//! Original `appletOE` / `appletAE` (Applet Manager) service implementation.
//! Manages game execution lifecycle, focus handling, and operation mode (docked/handheld).

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct AppletManagerService;

#[derive(Clone, Debug, Default)]
pub struct AppletSession;

impl Service for AppletSession {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetAppletResourceUserId -> returns u64
            1 => {
                let user_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: user_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetOperationMode -> returns u8 (0 = Handheld, 1 = Docked)
            21 => {
                let mode = 0u8; // Handheld default
                Ok(ServiceReply {
                    result: 0,
                    data: vec![mode],
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetPerformanceMode -> returns u32 (0 = Handheld, 1 = Docked)
            22 => {
                let perf_mode = 0u32;
                Ok(ServiceReply {
                    result: 0,
                    data: perf_mode.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "ILibraryAppletCreator",
                command,
            }),
        }
    }
}

impl Service for AppletManagerService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenLibraryAppletProxy / OpenApplicationProxy
            0 | 200 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(AppletSession::default())],
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "appletAE",
                command,
            }),
        }
    }
}
