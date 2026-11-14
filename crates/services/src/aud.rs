//! Original audio services (`audren:u`, `audout:u`).

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

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
            // GetAudioDeviceService (command 2)
            2 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(AudioOutManager::default())],
                    effect: None,
                })
            }
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
