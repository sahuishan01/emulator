//! Original `ectx:aw` (error context writer). Error contexts the guest
//! registers are counted and dropped; nothing is forwarded to the host.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

use crate::payload;

#[derive(Clone, Debug, Default)]
pub struct ErrorContextWriter;

impl Service for ErrorContextWriter {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // CreateContextRegistrar -> IContextRegistrar
            0 => {
                payload(request, 0)?;
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(ContextRegistrar::default())],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "ectx:aw",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ContextRegistrar;

impl Service for ContextRegistrar {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        Err(ServiceError::UnsupportedCommand {
            service: "ectx:IContextRegistrar",
            command: request.command_id,
        })
    }
}
