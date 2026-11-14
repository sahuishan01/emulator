//! Original `vi:m` (Visual Interface) service implementation.
//! Provides display creation, layer management, and buffer presentation endpoints.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct ViManagerService;

#[derive(Clone, Debug, Default)]
pub struct HosBinderDriver {
    connected: bool,
    current_slot: u32,
}

impl Service for HosBinderDriver {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // TransactParcel (0) & TransactParcelAuto (3)
            0 | 3 => {
                let binder_code = if request.data.len() >= 8 {
                    u32::from_le_bytes(request.data[4..8].try_into().unwrap_or([0; 4]))
                } else {
                    0
                };
                let mut reply_data = Vec::new();
                match binder_code {
                    // 1: REQUEST_BUFFER
                    1 => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&1u32.to_le_bytes()); // buffer present
                        reply_data.extend_from_slice(&1280u32.to_le_bytes()); // width
                        reply_data.extend_from_slice(&720u32.to_le_bytes());  // height
                        reply_data.extend_from_slice(&1280u32.to_le_bytes()); // stride
                        reply_data.extend_from_slice(&1u32.to_le_bytes());    // RGBA_8888
                        reply_data.extend_from_slice(&0xb00u32.to_le_bytes()); // usage
                    }
                    // 2: SET_BUFFER_COUNT
                    2 => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 3: DEQUEUE_BUFFER
                    3 => {
                        let slot = self.current_slot % 3;
                        self.current_slot = (self.current_slot + 1) % 3;
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&slot.to_le_bytes()); // slot
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // has_fence = 0
                    }
                    // 7: QUEUE_BUFFER
                    7 => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&1280u32.to_le_bytes()); // width
                        reply_data.extend_from_slice(&720u32.to_le_bytes());  // height
                        reply_data.extend_from_slice(&0u32.to_le_bytes());    // transform
                        reply_data.extend_from_slice(&0u32.to_le_bytes());    // pending buffers
                    }
                    // 8: CANCEL_BUFFER
                    8 => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 9: QUERY
                    9 => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 10: CONNECT
                    10 => {
                        self.connected = true;
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&1280u32.to_le_bytes());
                        reply_data.extend_from_slice(&720u32.to_le_bytes());
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 11: DISCONNECT
                    11 => {
                        self.connected = false;
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    _ => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                }
                Ok(ServiceReply {
                    result: 0,
                    data: reply_data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // AdjustRefcount (1)
            1 => Ok(ServiceReply::default()),
            // GetNativeHandle (2)
            2 => Ok(ServiceReply::default()),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IHOSBinderDriver",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DisplayService;

impl Service for DisplayService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetRelayService (command 100) -> returns IHOSBinderDriver
            100 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(HosBinderDriver::default())],
                    effect: None,
                })
            }
            // ListDisplays (command 1000)
            1000 => {
                let mut data = vec![0u8; 64];
                data[..7].copy_from_slice(b"Default");
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // OpenDisplay (command 1010) -> returns DisplayId (u64)
            1010 => {
                let display_id = 0u64;
                Ok(ServiceReply {
                    result: 0,
                    data: display_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // CloseDisplay (command 1011)
            1011 => Ok(ServiceReply::default()),
            // GetNaturalDisplayResolution (command 1020) -> returns width=1280 (u32), height=720 (u32)
            1020 => {
                let mut data = Vec::with_capacity(8);
                data.extend_from_slice(&1280u32.to_le_bytes());
                data.extend_from_slice(&720u32.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetDisplayMagnification (command 1101)
            1101 => Ok(ServiceReply::default()),
            // SetDisplayAlpha (command 1102)
            1102 => Ok(ServiceReply::default()),
            // CreateStrayLayer (command 2020) -> returns LayerId (u64), IGraphicBufferProducer parcel
            2020 => {
                let layer_id = 1u64;
                let mut data = Vec::with_capacity(16);
                data.extend_from_slice(&layer_id.to_le_bytes());
                data.extend_from_slice(&0u64.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // DestroyStrayLayer (command 2030)
            2030 => Ok(ServiceReply::default()),
            // CreateManagedLayer (command 2050) -> returns LayerId (u64)
            2050 => {
                let layer_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: layer_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // DestroyManagedLayer (command 2060)
            2060 => Ok(ServiceReply::default()),
            // SetLayerScalingMode (command 2101)
            2101 => Ok(ServiceReply::default()),
            // ConvertScalingMode (command 2102)
            2102 => Ok(ServiceReply::default()),
            // OpenSharedLayer (command 5202)
            5202 => {
                let data = vec![0u8; 16];
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IApplicationDisplayService",
                command,
            }),
        }
    }
}

impl Service for ViManagerService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetDisplayService (e.g. 0 = Application display service)
            0 | 1 | 2 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(DisplayService::default())],
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "vi:m",
                command,
            }),
        }
    }
}
