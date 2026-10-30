//! Original, deliberately small service implementations for owned homebrew.
use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

fn payload(request: &Request, length: usize) -> Result<(), ServiceError> {
    if request.data.len() == length {
        Ok(())
    } else {
        Err(ServiceError::InvalidPayload {
            expected: length,
            actual: request.data.len(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ServiceManager {
    initialized: bool,
}
impl Service for ServiceManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => {
                payload(request, 8)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                if self.initialized {
                    return Err(ServiceError::AlreadyInitialized);
                }
                self.initialized = true;
                Ok(ServiceReply::default())
            }
            1 => {
                payload(request, 8)?;
                if !self.initialized {
                    return Err(ServiceError::NotInitialized);
                }
                let name: [u8; 8] = request
                    .data
                    .as_slice()
                    .try_into()
                    .expect("validated payload");
                if &name != b"emu:test" {
                    return Err(ServiceError::UnsupportedService(name));
                }
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(DiagnosticCounter::default())],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "sm:",
                command,
            }),
        }
    }
}

/// An explicitly emulator-owned service, not a Nintendo service-success stub.
#[derive(Clone, Debug, Default)]
pub struct DiagnosticCounter {
    value: u64,
}
impl Service for DiagnosticCounter {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => {
                payload(request, 8)?;
                let increment = u64::from_le_bytes(
                    request
                        .data
                        .as_slice()
                        .try_into()
                        .expect("validated payload"),
                );
                self.value = self
                    .value
                    .checked_add(increment)
                    .ok_or(ServiceError::ArithmeticOverflow)?;
            }
            1 => payload(request, 0)?,
            command => {
                return Err(ServiceError::UnsupportedCommand {
                    service: "emu:test",
                    command,
                })
            }
        }
        Ok(ServiceReply {
            data: self.value.to_le_bytes().to_vec(),
            ..ServiceReply::default()
        })
    }
}
