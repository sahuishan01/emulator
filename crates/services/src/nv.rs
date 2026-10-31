//! Original standard nvdrv Initialize subset. It adopts a real process-owned
//! transfer arena; nvmap device metadata ioctls are bounded separately.
use crate::nvmap::{NvMap, BAD_PARAMETER, NOT_SUPPORTED, NO_MEMORY};
use emulator_ipc::Request;
use emulator_kernel::{
    service::{PreparedServiceEffect, Service, ServiceContext, ServiceError, ServiceReply},
    TransferMemory,
};
use emulator_memory::{BackingRange, MemoryView, Permissions};
use std::rc::Rc;
#[derive(Clone, Debug)]
pub struct NvDriver {
    arena: Option<Rc<TransferMemory>>,
    devices: Vec<Device>,
    next_fd: u32,
}
impl Default for NvDriver {
    fn default() -> Self {
        Self {
            arena: None,
            devices: Vec::new(),
            next_fd: 1,
        }
    }
}
#[derive(Clone, Debug)]
struct Device {
    fd: u32,
    nvmap: NvMap,
}
#[derive(Debug)]
struct BufferWrite {
    view: MemoryView,
    bytes: Vec<u8>,
    ranges: Vec<BackingRange>,
}
impl PreparedServiceEffect for BufferWrite {
    fn write_ranges(&self) -> &[BackingRange] {
        &self.ranges
    }
    fn commit(self: Box<Self>) {
        self.view
            .write(0, &self.bytes)
            .expect("prepared ioctl output pin");
    }
}
fn reply(words: &[u32]) -> ServiceReply {
    ServiceReply {
        data: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
        ..ServiceReply::default()
    }
}
fn input_word(request: &Request, offset: usize) -> u32 {
    u32::from_le_bytes(
        request.data[offset..offset + 4]
            .try_into()
            .expect("validated RPC word"),
    )
}
impl NvDriver {
    fn open(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(1, 0)?;
        super::payload(request, 0)?;
        if self.arena.is_none() {
            return Ok(reply(&[0, 3]));
        }
        let path = context.send_buffer(0)?;
        if path.is_empty() || path.len() > 64 {
            return Ok(reply(&[0, BAD_PARAMETER]));
        }
        let mut bytes = vec![0; path.len()];
        path.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        if bytes.iter().any(|b| !b.is_ascii() || *b == 0) {
            return Ok(reply(&[0, BAD_PARAMETER]));
        }
        if bytes != b"/dev/nvmap" {
            return Ok(reply(&[0, NOT_SUPPORTED]));
        }
        if self.devices.len() >= 16 {
            return Ok(reply(&[0, NO_MEMORY]));
        }
        let Some(next) = self
            .next_fd
            .checked_add(1)
            .filter(|v| *v <= i32::MAX as u32)
        else {
            return Ok(reply(&[0, NO_MEMORY]));
        };
        let fd = self.next_fd;
        self.devices.push(Device {
            fd,
            nvmap: NvMap::default(),
        });
        self.next_fd = next;
        Ok(reply(&[fd, 0]))
    }
    fn close(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(0, 0)?;
        super::payload(request, 4)?;
        if self.arena.is_none() {
            return Ok(reply(&[3]));
        }
        let fd = input_word(request, 0);
        let Some(at) = self.devices.iter().position(|d| d.fd == fd) else {
            return Ok(reply(&[BAD_PARAMETER]));
        };
        self.devices.remove(at);
        Ok(reply(&[0]))
    }
    fn ioctl(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(1, 1)?;
        super::payload(request, 8)?;
        if self.arena.is_none() {
            return Ok(reply(&[3]));
        }
        let fd = input_word(request, 0);
        let code = input_word(request, 4);
        let Some(device) = self.devices.iter_mut().find(|d| d.fd == fd) else {
            return Ok(reply(&[BAD_PARAMETER]));
        };
        let Some(size) = NvMap::argument_size(code) else {
            return Ok(reply(&[NOT_SUPPORTED]));
        };
        let input = context.send_buffer(0)?;
        let output = context.receive_buffer(0)?;
        if input.len() != size || output.len() != size {
            return Ok(reply(&[BAD_PARAMETER]));
        }
        let mut bytes = vec![0; size];
        input.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        match device.nvmap.ioctl(code, &bytes) {
            Err(error) => Ok(reply(&[error])),
            Ok(bytes) => {
                let ranges = output.backing_ranges().collect();
                let mut response = reply(&[0]);
                response.effect = Some(Box::new(BufferWrite {
                    view: output,
                    bytes,
                    ranges,
                }));
                Ok(response)
            }
        }
    }
}
impl Service for NvDriver {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        if request.command_id <= 3 {
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
            context.require_copy_handles(0)?;
            return match request.command_id {
                0 => self.open(request, context),
                1 => self.ioctl(request, context),
                2 => self.close(request, context),
                _ => self.dispatch(request),
            };
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
