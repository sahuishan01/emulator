//! Original `lm` (log manager) service. Guest log packets are accepted and
//! counted, never interpreted or forwarded to the host.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};

use crate::payload;

#[derive(Clone, Debug, Default)]
pub struct LogManager;

impl Service for LogManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenLogger(process id placeholder) -> ILogger
            0 => {
                payload(request, 8)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(Logger::default())],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "lm",
                command,
            }),
        }
    }
}

/// `ILogger`: a sink that only records how much the guest logged.
#[derive(Clone, Debug, Default)]
pub struct Logger {
    packets: u64,
    bytes: u64,
    destination: u32,
}
impl Logger {
    pub fn packets(&self) -> u64 {
        self.packets
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Service for Logger {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // SetDestination(u32 mask)
            1 => {
                payload(request, 4)?;
                self.destination =
                    u32::from_le_bytes(request.data.as_slice().try_into().expect("validated"));
                Ok(ServiceReply::default())
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "lm:ILogger",
                command,
            }),
        }
    }
    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        if request.command_id != 0 {
            context.require_buffers(0, 0)?;
            return self.dispatch(request);
        }
        // Log(auto-select input buffer holding one log packet)
        payload(request, 0)?;
        let packet = context.auto_send(0)?;
        self.packets = self.packets.saturating_add(1);
        self.bytes = self
            .bytes
            .saturating_add(packet.map_or(0, |view| view.len() as u64));
        Ok(ServiceReply::default())
    }
}
