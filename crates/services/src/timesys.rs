//! Original `time:s` / `time:u` (Time Service) implementation.
//! Provides steady clock and POSIX timestamps for Switch runtimes.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct TimeService;

impl Service for TimeService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetCurrentTime: returns POSIX seconds as u64
            0 => {
                let timestamp = 1767225600u64;
                Ok(ServiceReply {
                    result: 0,
                    data: timestamp.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "time",
                command,
            }),
        }
    }
}
