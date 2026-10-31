//! Original standard nvdrv Initialize subset. It adopts a real process-owned
//! transfer arena; device Open/Ioctl and other NV commands remain unsupported.
use emulator_ipc::Request;
use emulator_kernel::{
    service::{Service, ServiceContext, ServiceError, ServiceReply},
    TransferMemory,
};
use emulator_memory::Permissions;
use std::rc::Rc;
#[derive(Clone, Debug, Default)]
pub struct NvDriver {
    arena: Option<Rc<TransferMemory>>,
}
impl Service for NvDriver {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        if request.command_id == 3 {
            return Err(ServiceError::MissingMemoryContext);
        }
        Err(ServiceError::UnsupportedCommand {
            service: "nvdrv",
            command: request.command_id,
        })
    }
    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        if request.command_id != 3 {
            return self.dispatch(request);
        }
        context.require_buffers(0, 0)?;
        context.require_copy_handles(2)?;
        super::payload(request, 4)?;
        if self.arena.is_some() {
            return Err(ServiceError::AlreadyInitialized);
        }
        let size =
            u32::from_le_bytes(request.data.as_slice().try_into().expect("four bytes")) as usize;
        let arena = context.transfer_memory(0, 1)?;
        if size != arena.len() || arena.owner_permissions() != Permissions::NONE {
            return Err(ServiceError::InvalidTransferMemory);
        }
        self.arena = Some(arena);
        // The standard Initialize client wrapper expects a CMIF result only.
        Ok(ServiceReply::default())
    }
}
