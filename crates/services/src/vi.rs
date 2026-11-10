//! Original `vi:m` (Visual Interface) service implementation.
//! Provides display creation, layer management, and buffer presentation endpoints.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct ViManagerService;

#[derive(Clone, Debug, Default)]
pub struct DisplayService;

impl Service for DisplayService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenDisplay (e.g., "Default" display) -> returns DisplayId (u64)
            1010 => {
                let display_id = 0u64;
                Ok(ServiceReply {
                    result: 0,
                    data: display_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetNaturalDisplayResolution -> returns width=1280 (u32), height=720 (u32)
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
            // CreateStrayLayer -> returns LayerId (u64), IGraphicBufferProducer parcel
            2020 => {
                let layer_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: layer_id.to_le_bytes().to_vec(),
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
