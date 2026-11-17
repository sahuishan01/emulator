//! Original audio services (`audren:u`, `audout:u`).

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};

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

#[derive(Clone, Debug, Default)]
pub struct AudioOut;

impl Service for AudioOut {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetAudioOutState (command 0)
            0 => {
                let state = 0u32; // Started
                Ok(ServiceReply {
                    result: 0,
                    data: state.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // Start (command 1)
            1 => Ok(ServiceReply::default()),
            // Stop (command 2)
            2 => Ok(ServiceReply::default()),
            // AppendAudioOutBuffer (command 3)
            3 => Ok(ServiceReply::default()),
            // RegisterBufferEvent (command 4)
            4 => Ok(ServiceReply::default()),
            // GetReleasedAudioOutBuffers (command 5)
            5 => {
                let count = 0u32;
                Ok(ServiceReply {
                    result: 0,
                    data: count.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // ContainsAudioOutBuffer (command 6)
            6 => {
                let contains = 0u32;
                Ok(ServiceReply {
                    result: 0,
                    data: contains.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IAudioOut",
                command,
            }),
        }
    }
}

impl Service for AudioOutManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // ListAudioOuts (command 0)
            0 => {
                let mut data = vec![0u8; 64];
                data[..9].copy_from_slice(b"Headphone");
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // OpenAudioOut (command 1) -> returns sample_rate (48000), channel_count (2), sample_format (Pcm16), state (0)
            1 => {
                let mut data = Vec::with_capacity(32);
                data.extend_from_slice(&48000u32.to_le_bytes()); // sample_rate
                data.extend_from_slice(&2u32.to_le_bytes());     // channel_count
                data.extend_from_slice(&2u32.to_le_bytes());     // PcmInt16
                data.extend_from_slice(&0u32.to_le_bytes());     // State Started
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: vec![Box::new(AudioOut::default())],
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "audout:u",
                command,
            }),
        }
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
