//! Original `hwopus` (hardware Opus decoder manager).
//!
//! Opening decoders and sizing their work buffers is supported so titles can
//! initialise audio. There is no Opus bitstream decoder yet: every decode
//! command fails explicitly with `UnsupportedCommand` rather than returning
//! fabricated silence.

use std::rc::Rc;

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use emulator_kernel::TransferMemory;

/// Opus module 111, description 1 (invalid argument). Not hardware-verified;
/// it only has to be a non-zero failure.
pub const RESULT_OPUS_INVALID_ARGUMENT: u32 = 111 | (1 << 9);

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
        Ok(ServiceReply {
            moved_objects: vec![Box::new(OpusDecoder {
                params,
                _work: memory,
            })],
            ..ServiceReply::default()
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
            // Multi-stream (2, 3, 6, 7) and the rest are not implemented.
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
    params: OpusParameters,
    _work: Rc<TransferMemory>,
}

impl std::fmt::Debug for OpusDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpusDecoder")
            .field("params", &self.params)
            .finish()
    }
}

impl OpusDecoder {
    pub fn params(&self) -> OpusParameters {
        self.params
    }
}

impl Service for OpusDecoder {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        // Every command (DecodeInterleaved*, SetContext, ...) needs a real
        // Opus decoder, which does not exist yet.
        Err(ServiceError::UnsupportedCommand {
            service: "IHardwareOpusDecoder",
            command: request.command_id,
        })
    }
}
