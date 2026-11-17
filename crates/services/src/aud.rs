//! Original audio services (`audren:u`, `audout:u`).

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use emulator_memory::MemoryView;
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct AudioRendererManager;

#[derive(Clone, Debug, Default)]
pub struct AudioRenderer;

impl Service for AudioRenderer {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetAudioRendererState (command 0)
            0 => {
                let state = 0u32; // Started
                Ok(ServiceReply {
                    result: 0,
                    data: state.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // RequestUpdate (command 1)
            1 => {
                let mut data = vec![0u8; 0x40];
                data[0..4].copy_from_slice(&0u32.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // Start (command 2)
            2 => Ok(ServiceReply::default()),
            // Stop (command 3)
            3 => Ok(ServiceReply::default()),
            // QuerySystemEvent (command 4)
            4 => Ok(ServiceReply::default()),
            // SetAudioRendererRenderingTimeLimitPercent (command 5)
            5 => Ok(ServiceReply::default()),
            // GetAudioRendererRenderingTimeLimitPercent (command 6)
            6 => {
                let percent = 100u32;
                Ok(ServiceReply {
                    result: 0,
                    data: percent.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IAudioRenderer",
                command,
            }),
        }
    }
}

impl Service for AudioRendererManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenAudioRenderer (command 0) -> returns IAudioRenderer
            0 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(AudioRenderer::default())],
                    effect: None,
                })
            }
            // GetAudioRendererWorkBufferSize (command 1) -> returns size: u64
            1 => {
                let size = 0x4_0000u64; // 256 KiB work buffer
                Ok(ServiceReply {
                    result: 0,
                    data: size.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetAudioDeviceService (2) / GetAudioDeviceServiceWithRevisionInfo (4,
            // takes an applet resource user id and revision) -> IAudioDevice.
            2 | 4 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(AudioDevice::default())],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "audren:u",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AudioOutManager;

/// Guest `AudioOutBuffer` record: next, sample_buffer, capacity, data_size,
/// data_offset (five u64 fields).
const AUDIO_OUT_BUFFER_RECORD: usize = 0x28;
/// Horizon caps the buffers an audio out holds at once.
const AUDIO_OUT_MAX_BUFFERS: usize = 32;
/// PCM16 stereo frame size in bytes.
const AUDIO_OUT_FRAME_BYTES: u64 = 4;
const RESULT_AUDIO_BUFFER_COUNT_EXCEEDED: u32 = 151 | (153 << 9);

/// `IAudioOut` as a null sink. There is no host audio backend yet: samples
/// are discarded, and a buffer counts as played as soon as the out is
/// started (append while started, or Start with queued buffers). Release is
/// reported through the buffer event exactly as a real drain would, so
/// guests that pace on it progress; played-sample counts are not
/// wall-clock accurate.
#[derive(Clone, Debug)]
pub struct AudioOut {
    started: bool,
    /// Appended, not yet played (only while stopped).
    pending: VecDeque<(u64, u64)>,
    /// Played, not yet collected by GetReleasedAudioOutBuffers.
    released: VecDeque<u64>,
    played_samples: u64,
    volume: f32,
    event: Rc<Cell<bool>>,
}

impl Default for AudioOut {
    fn default() -> Self {
        Self {
            started: false,
            pending: VecDeque::new(),
            released: VecDeque::new(),
            played_samples: 0,
            volume: 1.0,
            event: Rc::new(Cell::new(false)),
        }
    }
}

impl AudioOut {
    fn drain_pending(&mut self) {
        if !self.started || self.pending.is_empty() {
            return;
        }
        for (tag, size) in self.pending.drain(..) {
            self.played_samples += size / AUDIO_OUT_FRAME_BYTES;
            self.released.push_back(tag);
        }
        self.event.set(true);
    }

    fn word<const N: usize>(request: &Request) -> Result<[u8; N], ServiceError> {
        request
            .data
            .get(..N)
            .map(|raw| raw.try_into().expect("N bytes"))
            .ok_or(ServiceError::InvalidPayload {
                expected: N,
                actual: request.data.len(),
            })
    }
}

impl Service for AudioOut {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        match request.command_id {
            // AppendAudioOutBuffer (3, A buffer) / Auto (7): u64 tag in, the
            // buffer holds the guest AudioOutBuffer record.
            3 | 7 => {
                let record = if request.command_id == 3 {
                    context.require_buffers(1, 0)?;
                    context.send_buffer(0).ok()
                } else {
                    context.require_auto_buffers(1, 0)?;
                    context.auto_send(0)?
                };
                let tag = u64::from_le_bytes(Self::word::<8>(request)?);
                let Some(record) = record.filter(|v| v.len() >= AUDIO_OUT_BUFFER_RECORD) else {
                    return Ok(ServiceReply {
                        result: RESULT_AUDIO_INVALID_ARGUMENT,
                        ..ServiceReply::default()
                    });
                };
                if self.pending.len() + self.released.len() >= AUDIO_OUT_MAX_BUFFERS {
                    return Ok(ServiceReply {
                        result: RESULT_AUDIO_BUFFER_COUNT_EXCEEDED,
                        ..ServiceReply::default()
                    });
                }
                let mut size = [0u8; 8];
                record.read(0x18, &mut size).map_err(ServiceError::Memory)?;
                self.pending.push_back((tag, u64::from_le_bytes(size)));
                self.drain_pending();
                Ok(ServiceReply::default())
            }
            // GetReleasedAudioOutBuffers (5, B buffer) / Auto (8): u64 tags
            // out, count returned.
            5 | 8 => {
                let output = if request.command_id == 5 {
                    context.require_buffers(0, 1)?;
                    context.receive_buffer(0).ok()
                } else {
                    context.require_auto_buffers(0, 1)?;
                    context.auto_receive(0)?
                };
                let room = output.as_ref().map_or(0, |v| v.len() / 8);
                let count = room.min(self.released.len());
                let tags: Vec<u8> = self
                    .released
                    .drain(..count)
                    .flat_map(u64::to_le_bytes)
                    .collect();
                if self.released.is_empty() {
                    self.event.set(false);
                }
                Ok(ServiceReply {
                    data: (count as u32).to_le_bytes().to_vec(),
                    effect: output
                        .filter(|_| count != 0)
                        .map(|v| Box::new(crate::BufferWrite::new(v, tags)) as _),
                    ..ServiceReply::default()
                })
            }
            _ => {
                context.require_buffers(0, 0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        let reply = |data: Vec<u8>| ServiceReply {
            data,
            ..ServiceReply::default()
        };
        match request.command_id {
            // GetAudioOutState: 0 started, 1 stopped.
            0 => Ok(reply(u32::from(!self.started).to_le_bytes().to_vec())),
            1 => {
                self.started = true;
                self.drain_pending();
                Ok(reply(Vec::new()))
            }
            2 => {
                self.started = false;
                Ok(reply(Vec::new()))
            }
            // RegisterBufferEvent: the shared release event.
            4 => Ok(ServiceReply {
                moved_objects: vec![Box::new(crate::event::EventHandle(Rc::clone(&self.event)))],
                ..ServiceReply::default()
            }),
            // ContainsAudioOutBuffer: still owned by the audio out.
            6 => {
                let tag = u64::from_le_bytes(Self::word::<8>(request)?);
                let held = self.pending.iter().any(|&(t, _)| t == tag) || self.released.contains(&tag);
                Ok(reply(vec![u8::from(held)]))
            }
            // GetAudioOutBufferCount
            9 => Ok(reply(((self.pending.len() + self.released.len()) as u32).to_le_bytes().to_vec())),
            // GetAudioOutPlayedSampleCount
            10 => Ok(reply(self.played_samples.to_le_bytes().to_vec())),
            // FlushAudioOutBuffers: queued buffers are returned unplayed.
            11 => {
                let any = !self.pending.is_empty();
                for (tag, _) in self.pending.drain(..) {
                    self.released.push_back(tag);
                }
                if any {
                    self.event.set(true);
                }
                Ok(reply(vec![u8::from(any)]))
            }
            12 => {
                let volume = f32::from_le_bytes(Self::word::<4>(request)?);
                if !(0.0..=1.0).contains(&volume) {
                    return Ok(ServiceReply {
                        result: RESULT_AUDIO_INVALID_ARGUMENT,
                        ..ServiceReply::default()
                    });
                }
                self.volume = volume;
                Ok(reply(Vec::new()))
            }
            13 => Ok(reply(self.volume.to_le_bytes().to_vec())),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IAudioOut",
                command,
            }),
        }
    }
}

/// Audio-out names are fixed 0x100-byte NUL-padded records.
const AUDIO_OUT_NAME_LEN: usize = 0x100;
/// The single audio output exposed by `audout:u`.
const DEFAULT_AUDIO_OUT: &[u8] = b"DeviceOut";
const AUDIO_OUT_SAMPLE_RATE: u32 = 48_000;
const PCM_FORMAT_INT16: u32 = 2;
const AUDIO_OUT_STATE_STOPPED: u32 = 1;

fn read_name(view: Option<MemoryView>) -> Result<Vec<u8>, ServiceError> {
    let Some(view) = view else {
        return Ok(Vec::new());
    };
    let mut raw = vec![0u8; view.len().min(AUDIO_OUT_NAME_LEN)];
    view.read(0, &mut raw).map_err(ServiceError::Memory)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    raw.truncate(end);
    Ok(raw)
}

fn padded_out_name() -> Vec<u8> {
    let mut name = vec![0u8; AUDIO_OUT_NAME_LEN];
    name[..DEFAULT_AUDIO_OUT.len()].copy_from_slice(DEFAULT_AUDIO_OUT);
    name
}

impl Service for AudioOutManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // ListAudioOuts (0, B buffer) / ListAudioOutsAuto (2): write the
            // one output name and return the count written.
            0 | 2 => {
                context.require_copy_handles(0)?;
                let output = if request.command_id == 0 {
                    context.require_buffers(0, 1)?;
                    Some(context.receive_buffer(0)?)
                } else {
                    context.require_auto_buffers(0, 1)?;
                    context.auto_receive(0)?
                };
                match output.filter(|v| v.len() >= AUDIO_OUT_NAME_LEN) {
                    Some(output) => Ok(ServiceReply {
                        data: 1u32.to_le_bytes().to_vec(),
                        effect: Some(Box::new(crate::BufferWrite::new(output, padded_out_name()))),
                        ..ServiceReply::default()
                    }),
                    None => Ok(ServiceReply {
                        data: 0u32.to_le_bytes().to_vec(),
                        ..ServiceReply::default()
                    }),
                }
            }
            // OpenAudioOut (1, A + B buffers) / OpenAudioOutAuto (3). Input:
            // sample_rate u32, channel_count u16, reserved u16, ARUID u64,
            // plus the client process handle. An empty name selects the
            // default output.
            1 | 3 => {
                if request.copy_handles.len() > 1 {
                    return Err(ServiceError::InvalidCopyHandleCount(request.copy_handles.len()));
                }
                let (input, output) = if request.command_id == 1 {
                    context.require_buffers(1, 1)?;
                    (context.send_buffer(0).ok(), context.receive_buffer(0).ok())
                } else {
                    context.require_auto_buffers(1, 1)?;
                    (context.auto_send(0)?, context.auto_receive(0)?)
                };
                let data = request.data.get(..16).ok_or(ServiceError::InvalidPayload {
                    expected: 16,
                    actual: request.data.len(),
                })?;
                let sample_rate = u32::from_le_bytes(data[0..4].try_into().expect("4"));
                let channels = u16::from_le_bytes(data[4..6].try_into().expect("2"));
                let name = read_name(input)?;
                let invalid = ServiceReply {
                    result: RESULT_AUDIO_INVALID_ARGUMENT,
                    ..ServiceReply::default()
                };
                if !(name.is_empty() || name == DEFAULT_AUDIO_OUT)
                    || !(sample_rate == 0 || sample_rate == AUDIO_OUT_SAMPLE_RATE)
                    || channels > 2
                {
                    // Only the 48 kHz stereo default output exists; 6-channel
                    // output is not implemented and is rejected, not faked.
                    return Ok(invalid);
                }
                let mut reply = Vec::with_capacity(16);
                reply.extend_from_slice(&AUDIO_OUT_SAMPLE_RATE.to_le_bytes());
                reply.extend_from_slice(&2u32.to_le_bytes());
                reply.extend_from_slice(&PCM_FORMAT_INT16.to_le_bytes());
                reply.extend_from_slice(&AUDIO_OUT_STATE_STOPPED.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data: reply,
                    moved_objects: vec![Box::new(AudioOut::default())],
                    effect: output
                        .filter(|v| v.len() >= AUDIO_OUT_NAME_LEN)
                        .map(|v| Box::new(crate::BufferWrite::new(v, padded_out_name())) as _),
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "audout:u",
                command,
            }),
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        Err(ServiceError::UnsupportedCommand {
            service: "audout:u",
            command: request.command_id,
        })
    }
}

/// `IAudioDevice`: one fixed stereo output device at unit volume.
#[derive(Clone, Debug)]
pub struct AudioDevice {
    volume: f32,
}

impl Default for AudioDevice {
    fn default() -> Self {
        Self { volume: 1.0 }
    }
}

/// Device names are fixed 0x100-byte NUL-padded records.
const AUDIO_DEVICE_NAME_LEN: usize = 0x100;
/// The single output we expose (handheld speaker).
const ACTIVE_AUDIO_DEVICE: &[u8] = b"AudioBuiltInSpeakerOutput";

impl Service for AudioDevice {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        // Name commands: 0 List / 3 GetActive use B buffers; 6 / 10 / 13 are
        // the auto-select variants. 1 / 7 (set volume) send a name buffer
        // that is not inspected.
        let output = match request.command_id {
            0 | 3 => {
                context.require_buffers(0, 1)?;
                Some(context.receive_buffer(0)?)
            }
            6 | 10 | 13 => {
                context.require_auto_buffers(0, 1)?;
                context.auto_receive(0)?
            }
            1 => {
                context.require_buffers(1, 0)?;
                return self.dispatch(request);
            }
            7 => {
                context.require_auto_buffers(1, 0)?;
                return self.dispatch(request);
            }
            _ => {
                context.require_buffers(0, 0)?;
                return self.dispatch(request);
            }
        };
        let Some(output) = output.filter(|v| v.len() >= AUDIO_DEVICE_NAME_LEN) else {
            return Ok(ServiceReply {
                result: RESULT_AUDIO_INVALID_ARGUMENT,
                ..ServiceReply::default()
            });
        };
        let mut name = vec![0u8; AUDIO_DEVICE_NAME_LEN];
        name[..ACTIVE_AUDIO_DEVICE.len()].copy_from_slice(ACTIVE_AUDIO_DEVICE);
        // List commands return the number of names written.
        let data = if matches!(request.command_id, 0 | 6) {
            1u32.to_le_bytes().to_vec()
        } else {
            Vec::new()
        };
        Ok(ServiceReply {
            data,
            effect: Some(Box::new(crate::BufferWrite::new(output, name))),
            ..ServiceReply::default()
        })
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        let reply = |data: Vec<u8>| ServiceReply {
            data,
            ..ServiceReply::default()
        };
        match request.command_id {
            // SetAudioDeviceOutputVolume / SetAudioDeviceOutputVolumeAuto: the
            // volume is the first data word (f32); the device name buffer is
            // not inspected because only one device exists.
            1 | 7 => {
                let raw = request.data.get(..4).ok_or(ServiceError::InvalidPayload {
                    expected: 4,
                    actual: request.data.len(),
                })?;
                let volume = f32::from_le_bytes(raw.try_into().expect("4 bytes"));
                if !(0.0..=1.0).contains(&volume) {
                    return Ok(ServiceReply {
                        result: RESULT_AUDIO_INVALID_ARGUMENT,
                        ..ServiceReply::default()
                    });
                }
                self.volume = volume;
                Ok(reply(Vec::new()))
            }
            // GetAudioDeviceOutputVolume / ...Auto
            2 | 8 => Ok(reply(self.volume.to_le_bytes().to_vec())),
            // QueryAudioDeviceSystemEvent / InputEvent / OutputEvent: no device
            // changes happen, so the events never signal.
            4 | 11 | 12 => Ok(ServiceReply {
                moved_objects: vec![Box::new(crate::event::EventHandle::new(false))],
                ..ServiceReply::default()
            }),
            // GetActiveChannelCount
            5 => Ok(reply(2u32.to_le_bytes().to_vec())),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IAudioDevice",
                command,
            }),
        }
    }
}

/// Out-of-range volume result: audio module 153, description 41. The exact
/// description is not verified against hardware; it only has to be non-zero.
const RESULT_AUDIO_INVALID_ARGUMENT: u32 = 153 | (41 << 9);
