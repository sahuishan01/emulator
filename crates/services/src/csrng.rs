//! Original `csrng` (cryptographically secure random numbers) service. Bytes
//! come from the host kernel's CSPRNG (`/dev/urandom`, present on Linux and
//! Android); if it cannot be read the request fails instead of returning
//! predictable data.

use crate::BufferWrite;
use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use std::io::Read;

/// Fills `bytes` from the host CSPRNG.
pub fn host_random(bytes: &mut [u8]) -> Result<(), ServiceError> {
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(bytes))
        .map_err(|error| ServiceError::DeviceFault(format!("host entropy unavailable: {error}")))
}

#[derive(Clone, Debug, Default)]
pub struct RandomService;

impl Service for RandomService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
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
        // GenerateRandomBytes(B buffer): fills the whole buffer.
        crate::payload(request, 0)?;
        context.require_exact_buffers(0, 1, 0, 0)?;
        let view = context.receive_buffer(0)?;
        let mut bytes = vec![0u8; view.len()];
        host_random(&mut bytes)?;
        Ok(ServiceReply {
            effect: Some(Box::new(BufferWrite::new(view, bytes))),
            ..ServiceReply::default()
        })
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        Err(ServiceError::UnsupportedCommand {
            service: "csrng",
            command: request.command_id,
        })
    }
}
