//! Original `apm` (Performance Management) service implementation.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct ApmService;

#[derive(Clone, Debug, Default)]
pub struct ApmSession;

impl Service for ApmSession {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // SetPerformanceConfiguration (command 0)
            0 => Ok(ServiceReply::default()),
            // GetPerformanceConfiguration (command 1)
            1 => {
                let config = 0x0001_0001u32;
                Ok(ServiceReply {
                    result: 0,
                    data: config.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "ISession",
                command,
            }),
        }
    }
}

impl Service for ApmService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenSession (command 0) -> returns ISession
            0 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(ApmSession::default())],
                    effect: None,
                })
            }
            // GetPerformanceMode (command 1) -> returns 0 (Handheld) or 1 (Docked)
            1 => {
                let mode = 0u32; // Handheld mode
                Ok(ServiceReply {
                    result: 0,
                    data: mode.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "apm",
                command,
            }),
        }
    }
}
