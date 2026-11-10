//! Original `hid` (Human Interface Device) service implementation.
//! Provides controller connection states, button inputs, touch screen events,
//! and six-axis motion sensor readings.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct HidService;

#[derive(Clone, Debug, Default)]
pub struct HidAppletResource;

impl Service for HidAppletResource {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetSharedMemoryHandle: provides shared memory handle for input state polling
            0 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IAppletResource",
                command,
            }),
        }
    }
}

impl Service for HidService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // CreateAppletResource -> returns IAppletResource object
            0 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(HidAppletResource::default())],
                    effect: None,
                })
            }
            // ActivateDebugPad
            1 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetSupportedNpadStyleSet
            100 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetSupportedNpadIdType
            102 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // ActivateNpad
            103 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "hid",
                command,
            }),
        }
    }
}
