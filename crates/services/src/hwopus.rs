//! Original `hwopus` (hardware Opus decoder manager).
//!
//! Single-stream decoders are backed by the project's own Opus decoder
//! (`emulator-opus`). Guests submit packets with an 8-byte big-endian
//! header `{payload size, encoder final range}` and receive interleaved
//! 16-bit PCM at the rate they opened the decoder with. Multi-stream
//! decoders (RFC 7845 channel mapping) take their parameters from an X
//! buffer and use the same packet framing. The guest work buffer is retained
//! for the decoder's lifetime but all decoder state is host side.
//! `SetContext` still fails explicitly.

use std::rc::Rc;

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use emulator_kernel::TransferMemory;
use emulator_opus::{Decoder, Error as OpusError, MultiStreamDecoder};

use crate::BufferWrite;

const HWOPUS: u32 = 111;
const fn hwopus_result(description: u32) -> u32 {
    HWOPUS | (description << 9)
}

/// Opus module 111, description 1 (invalid argument). Not hardware-verified;
/// it only has to be a non-zero failure.
pub const RESULT_OPUS_INVALID_ARGUMENT: u32 = hwopus_result(1);
/// libopus `OPUS_BAD_ARG` as reported by the hardware decoder.
pub const RESULT_OPUS_BAD_ARG: u32 = hwopus_result(2);
/// Output buffer too small, or header size larger than the input buffer.
pub const RESULT_OPUS_BUFFER_TOO_SMALL: u32 = hwopus_result(3);
pub const RESULT_OPUS_INTERNAL_ERROR: u32 = hwopus_result(4);
/// Input buffer not larger than the 8-byte packet header.
pub const RESULT_OPUS_INPUT_TOO_SMALL: u32 = hwopus_result(8);
/// Malformed packet, or the header's final range does not match.
pub const RESULT_OPUS_INVALID_PACKET: u32 = hwopus_result(17);

const PACKET_HEADER_SIZE: usize = 8;

const SAMPLE_RATES: [u32; 5] = [8_000, 12_000, 16_000, 24_000, 48_000];

/// Validated single-stream decoder parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpusParameters {
    pub sample_rate: u32,
    pub channel_count: u32,
    pub large_frames: bool,
}

impl OpusParameters {
    fn parse(data: &[u8], ex: bool) -> Option<Self> {
        let word = |i: usize| Some(u32::from_le_bytes(data.get(i..i + 4)?.try_into().ok()?));
        let params = Self {
            sample_rate: word(0)?,
            channel_count: word(4)?,
            large_frames: if ex { *data.get(8)? != 0 } else { false },
        };
        (SAMPLE_RATES.contains(&params.sample_rate) && matches!(params.channel_count, 1 | 2))
            .then_some(params)
    }

    /// Our own work-buffer budget: per-channel decoder state plus a PCM
    /// staging area for one maximal frame (120 ms with large frames, 20 ms
    /// otherwise) at 48 kHz, page aligned.
    pub fn work_buffer_size(self) -> u32 {
        let frame_ms = if self.large_frames { 120 } else { 20 };
        let pcm = 48 * frame_ms * 2 * self.channel_count;
        let state = 0x4000 * self.channel_count;
        (0x1000 + state + pcm + 0xfff) & !0xfff
    }
}

/// `OpusMultiStreamParameters` (0x110 bytes) and its Ex form (0x118 bytes,
/// `use_large_frame_size` and 7 padding bytes before the mapping).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpusMultiStreamParameters {
    pub sample_rate: u32,
    pub channel_count: u32,
    pub stream_count: u32,
    pub stereo_stream_count: u32,
    pub large_frames: bool,
    pub mapping: Vec<u8>,
}

/// Size of the channel mapping table in the parameter structs.
pub const MULTISTREAM_MAPPING_SIZE: usize = 0x100;

impl OpusMultiStreamParameters {
    pub fn wire_size(ex: bool) -> usize {
        if ex {
            0x18 + MULTISTREAM_MAPPING_SIZE
        } else {
            0x10 + MULTISTREAM_MAPPING_SIZE
        }
    }

    /// Wire bytes of these parameters (the mapping is zero padded).
    pub fn to_bytes(&self, ex: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::wire_size(ex));
        for v in [
            self.sample_rate,
            self.channel_count,
            self.stream_count,
            self.stereo_stream_count,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        if ex {
            out.push(u8::from(self.large_frames));
            out.extend_from_slice(&[0; 7]);
        }
        let mut mapping = self.mapping.clone();
        mapping.resize(MULTISTREAM_MAPPING_SIZE, 0);
        out.extend_from_slice(&mapping);
        out
    }

    /// Validated parameters; `None` for anything the decoder cannot build
    /// (rates, 1..=255 channels, stream counts, mapping entries).
    fn parse(data: &[u8], ex: bool) -> Option<Self> {
        if data.len() < Self::wire_size(ex) {
            return None;
        }
        let word = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().expect("4 bytes"));
        let mapping_at = if ex { 0x18 } else { 0x10 };
        let params = Self {
            sample_rate: word(0),
            channel_count: word(4),
            stream_count: word(8),
            stereo_stream_count: word(12),
            large_frames: ex && data[0x10] != 0,
            mapping: data[mapping_at..mapping_at + word(4).min(255) as usize].to_vec(),
        };
        (SAMPLE_RATES.contains(&params.sample_rate) && params.decoder().is_ok()).then_some(params)
    }

    fn decoder(&self) -> Result<MultiStreamDecoder, OpusError> {
        MultiStreamDecoder::new(
            self.sample_rate,
            self.channel_count as usize,
            self.stream_count as usize,
            self.stereo_stream_count as usize,
            &self.mapping,
        )
    }

    /// Our own budget: decoder state per coded channel plus one maximal
    /// frame of PCM for every output channel, page aligned.
    pub fn work_buffer_size(&self) -> u32 {
        let frame_ms = if self.large_frames { 120 } else { 20 };
        let pcm = 48 * frame_ms * 2 * self.channel_count;
        let state = 0x4000 * (self.stream_count + self.stereo_stream_count);
        (0x1000 + state + pcm + 0xfff) & !0xfff
    }
}

/// Read the multi-stream parameters from X buffer 0.
fn multistream_params(
    context: &ServiceContext<'_>,
    ex: bool,
) -> Result<Option<OpusMultiStreamParameters>, ServiceError> {
    let size = OpusMultiStreamParameters::wire_size(ex);
    let Some(view) = context.pointer_send(0)? else {
        return Ok(None);
    };
    if view.len() < size {
        return Ok(None);
    }
    let mut bytes = vec![0u8; size];
    view.read(0, &mut bytes).map_err(ServiceError::Memory)?;
    Ok(OpusMultiStreamParameters::parse(&bytes, ex))
}

fn invalid() -> ServiceReply {
    ServiceReply {
        result: RESULT_OPUS_INVALID_ARGUMENT,
        ..ServiceReply::default()
    }
}

fn words(values: &[u32]) -> ServiceReply {
    ServiceReply {
        data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        ..ServiceReply::default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct OpusDecoderManager;

impl OpusDecoderManager {
    /// 0 OpenHardwareOpusDecoder: {rate, channels, size} + transfer memory.
    /// 4 OpenHardwareOpusDecoderEx: {rate, channels, large, pad[7]} then size.
    fn open(
        request: &Request,
        context: &ServiceContext<'_>,
        ex: bool,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(0, 0)?;
        let memory = context.sole_transfer_memory()?;
        let size_offset = if ex { 0x10 } else { 8 };
        let size =
            request
                .data
                .get(size_offset..size_offset + 4)
                .ok_or(ServiceError::InvalidPayload {
                    expected: size_offset + 4,
                    actual: request.data.len(),
                })?;
        let size = u32::from_le_bytes(size.try_into().expect("4 bytes"));
        let Some(params) = OpusParameters::parse(&request.data, ex) else {
            return Ok(invalid());
        };
        if (size as usize) < params.work_buffer_size() as usize || memory.len() < size as usize {
            return Ok(invalid());
        }
        let Ok(decoder) = Decoder::new(params.sample_rate, params.channel_count as usize) else {
            return Ok(invalid());
        };
        Ok(ServiceReply {
            moved_objects: vec![Box::new(OpusDecoder {
                sample_rate: params.sample_rate,
                channels: params.channel_count as usize,
                _work: memory,
                engine: Engine::Single(decoder),
            })],
            ..ServiceReply::default()
        })
    }
}

impl OpusDecoderManager {
    /// 2 OpenHardwareOpusDecoderForMultiStream / 6 ...Ex: X = parameters,
    /// raw {work buffer size}, copied transfer memory.
    fn open_multistream(
        request: &Request,
        context: &ServiceContext<'_>,
        ex: bool,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_exact_buffers(0, 0, 1, 0)?;
        let memory = context.sole_transfer_memory()?;
        let size = request.data.get(..4).ok_or(ServiceError::InvalidPayload {
            expected: 4,
            actual: request.data.len(),
        })?;
        let size = u32::from_le_bytes(size.try_into().expect("4 bytes"));
        let Some(params) = multistream_params(context, ex)? else {
            return Ok(invalid());
        };
        if (size as usize) < params.work_buffer_size() as usize || memory.len() < size as usize {
            return Ok(invalid());
        }
        let Ok(decoder) = params.decoder() else {
            return Ok(invalid());
        };
        Ok(ServiceReply {
            moved_objects: vec![Box::new(OpusDecoder {
                sample_rate: params.sample_rate,
                channels: params.channel_count as usize,
                _work: memory,
                engine: Engine::Multi(decoder),
            })],
            ..ServiceReply::default()
        })
    }

    /// 3 / 7 / 9 GetWorkBufferSizeForMultiStream(Ex/ExEx): X = parameters.
    fn multistream_size(
        context: &ServiceContext<'_>,
        ex: bool,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        context.require_exact_buffers(0, 0, 1, 0)?;
        Ok(match multistream_params(context, ex)? {
            Some(params) => words(&[params.work_buffer_size()]),
            None => invalid(),
        })
    }
}

impl Service for OpusDecoderManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => Self::open(request, context, false),
            4 => Self::open(request, context, true),
            2 => Self::open_multistream(request, context, false),
            6 => Self::open_multistream(request, context, true),
            3 => Self::multistream_size(context, false),
            7 | 9 => Self::multistream_size(context, true),
            _ => {
                context.require_copy_handles(0)?;
                context.require_buffers(0, 0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetWorkBufferSize (1) / GetWorkBufferSizeEx (5) /
            // GetWorkBufferSizeExEx (8, same OpusParametersEx input).
            1 | 5 | 8 => Ok(
                match OpusParameters::parse(&request.data, request.command_id != 1) {
                    Some(params) => words(&[params.work_buffer_size()]),
                    None => invalid(),
                },
            ),
            // Everything else is not implemented.
            command => Err(ServiceError::UnsupportedCommand {
                service: "hwopus",
                command,
            }),
        }
    }
}

/// `IHardwareOpusDecoder`. Holds the guest work buffer for its lifetime.
#[derive(Clone)]
pub struct OpusDecoder {
    sample_rate: u32,
    channels: usize,
    _work: Rc<TransferMemory>,
    engine: Engine,
}

#[derive(Clone)]
enum Engine {
    Single(Decoder),
    Multi(MultiStreamDecoder),
}

impl Engine {
    fn reset(&mut self) {
        match self {
            Self::Single(d) => d.reset(),
            Self::Multi(d) => d.reset(),
        }
    }
    fn decode(
        &mut self,
        packet: &[u8],
        pcm: &mut [i16],
        capacity: usize,
    ) -> Result<usize, OpusError> {
        match self {
            Self::Single(d) => d.decode(Some(packet), pcm, capacity),
            Self::Multi(d) => d.decode(Some(packet), pcm, capacity),
        }
    }
    fn final_range(&self) -> u32 {
        match self {
            Self::Single(d) => d.final_range(),
            Self::Multi(d) => d.final_range(),
        }
    }
}

impl std::fmt::Debug for OpusDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpusDecoder")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("multistream", &matches!(self.engine, Engine::Multi(_)))
            .finish()
    }
}

fn failure(result: u32) -> ServiceReply {
    ServiceReply {
        result,
        ..ServiceReply::default()
    }
}

fn opus_result(error: OpusError) -> u32 {
    match error {
        OpusError::BadArgument => RESULT_OPUS_BAD_ARG,
        OpusError::BufferTooSmall => RESULT_OPUS_BUFFER_TOO_SMALL,
        OpusError::InternalError => RESULT_OPUS_INTERNAL_ERROR,
        OpusError::InvalidPacket => RESULT_OPUS_INVALID_PACKET,
    }
}

impl OpusDecoder {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channel_count(&self) -> usize {
        self.channels
    }

    pub fn is_multistream(&self) -> bool {
        matches!(self.engine, Engine::Multi(_))
    }

    /// DecodeInterleaved* (0, 4, 6, 8): A = header + packet, B = PCM.
    /// Replies {consumed bytes, samples per channel} plus a modelled decode
    /// time in microseconds for the perf variants.
    fn decode(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
        with_reset: bool,
        with_perf: bool,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        context.require_buffers(1, 1)?;
        let reset = if with_reset {
            *request.data.first().ok_or(ServiceError::InvalidPayload {
                expected: 1,
                actual: request.data.len(),
            })? != 0
        } else {
            false
        };
        let input = context.send_buffer(0)?;
        let output = context.receive_buffer(0)?;
        if input.len() <= PACKET_HEADER_SIZE {
            return Ok(failure(RESULT_OPUS_INPUT_TOO_SMALL));
        }
        let mut header = [0u8; PACKET_HEADER_SIZE];
        input.read(0, &mut header).map_err(ServiceError::Memory)?;
        let size = u32::from_be_bytes(header[..4].try_into().expect("4 bytes")) as usize;
        let final_range = u32::from_be_bytes(header[4..].try_into().expect("4 bytes"));
        if size > input.len() - PACKET_HEADER_SIZE {
            return Ok(failure(RESULT_OPUS_BUFFER_TOO_SMALL));
        }
        let mut packet = vec![0u8; size];
        input
            .read(PACKET_HEADER_SIZE, &mut packet)
            .map_err(ServiceError::Memory)?;
        let channels = self.channels;
        let capacity = output.len() / (2 * channels);
        if reset {
            self.engine.reset();
        }
        let mut pcm = vec![0i16; capacity * channels];
        let samples = match self.engine.decode(&packet, &mut pcm, capacity) {
            Ok(samples) => samples,
            Err(error) => return Ok(failure(opus_result(error))),
        };
        if final_range != 0 && size > 0 && self.engine.final_range() != final_range {
            return Ok(failure(RESULT_OPUS_INVALID_PACKET));
        }
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&((size + PACKET_HEADER_SIZE) as u32).to_le_bytes());
        data.extend_from_slice(&(samples as u32).to_le_bytes());
        if with_perf {
            // Modelled DSP cost: 1% of the decoded duration.
            let micros = samples as u64 * 10_000 / u64::from(self.sample_rate);
            data.extend_from_slice(&micros.to_le_bytes());
        }
        let bytes: Vec<u8> = pcm[..samples * channels]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut reply = ServiceReply {
            data,
            ..ServiceReply::default()
        };
        if !bytes.is_empty() {
            let view = output.slice(0, bytes.len()).map_err(ServiceError::Memory)?;
            reply.effect = Some(Box::new(BufferWrite::new(view, bytes)));
        }
        Ok(reply)
    }
}

impl Service for OpusDecoder {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        // Single-stream commands 0/4/6/8, multi-stream 2/5/7/9 (Old, WithPerfOld,
        // WithPerfAndResetOld, current). Each needs the matching decoder kind.
        let (multistream, reset, perf) = match request.command_id {
            0 => (false, false, false),
            4 => (false, false, true),
            6 | 8 => (false, true, true),
            2 => (true, false, false),
            5 => (true, false, true),
            7 | 9 => (true, true, true),
            _ => return self.dispatch(request),
        };
        if multistream != self.is_multistream() {
            // Single-stream commands on a multi-stream decoder and vice versa.
            return Err(ServiceError::UnsupportedCommand {
                service: "IHardwareOpusDecoder",
                command: request.command_id,
            });
        }
        self.decode(request, context, reset, perf)
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        // SetContext (1) / SetContextForMultiStream (3) are not implemented.
        Err(ServiceError::UnsupportedCommand {
            service: "IHardwareOpusDecoder",
            command: request.command_id,
        })
    }
}
