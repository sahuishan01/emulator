//! Original standard nvdrv Initialize subset. It adopts a real process-owned
//! transfer arena; nvmap device metadata ioctls are bounded separately.
use crate::nvas::NvAddressSpace;
use crate::nvmap::{
    ImportBudget, NvMap, BAD_PARAMETER, IMPORT_BYTES, IMPORT_SPANS, NOT_SUPPORTED, NO_MEMORY,
};
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
    nvmap: NvMap,
}
impl Default for NvDriver {
    fn default() -> Self {
        Self {
            arena: None,
            devices: Vec::new(),
            next_fd: 1,
            nvmap: NvMap::default(),
        }
    }
}
#[derive(Clone, Debug)]
struct Device {
    fd: u32,
    object: DeviceObject,
}
#[derive(Clone, Debug)]
enum DeviceObject {
    NvMap,
    AddressSpace(NvAddressSpace),
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
        let object = match bytes.as_slice() {
            b"/dev/nvmap" => DeviceObject::NvMap,
            b"/dev/nvhost-as-gpu" => DeviceObject::AddressSpace(NvAddressSpace::default()),
            _ => return Ok(reply(&[0, NOT_SUPPORTED])),
        };
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
        self.devices.push(Device { fd, object });
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
        if matches!(self.devices[at].object, DeviceObject::NvMap) {
            self.nvmap.close_device(fd);
        }
        self.devices.remove(at);
        Ok(reply(&[0]))
    }
    fn ioctl(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        super::payload(request, 8)?;
        let fd = input_word(request, 0);
        let code = input_word(request, 4);
        let send = usize::from(code & (1 << 30) != 0);
        let receive = usize::from(code & (1 << 31) != 0);
        context.require_buffers(send, receive)?;
        if self.arena.is_none() {
            return Ok(reply(&[3]));
        }
        let Some(at) = self.devices.iter().position(|d| d.fd == fd) else {
            return Ok(reply(&[BAD_PARAMETER]));
        };
        let size = match &self.devices[at].object {
            DeviceObject::NvMap => NvMap::argument_size(code),
            DeviceObject::AddressSpace(_) => NvAddressSpace::argument_size(code),
        };
        let Some(size) = size else {
            return Ok(reply(&[NOT_SUPPORTED]));
        };
        let input = context.send_buffer(0)?;
        let output = if receive == 1 {
            Some(context.receive_buffer(0)?)
        } else {
            None
        };
        if input.len() != size || output.as_ref().is_some_and(|v| v.len() != size) {
            return Ok(reply(&[BAD_PARAMETER]));
        }
        let mut bytes = vec![0; size];
        input.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        let handle = if code == 0xc0180105 {
            u32::from_le_bytes(bytes[..4].try_into().expect("free handle"))
        } else {
            0
        };
        let retained = self
            .devices
            .iter()
            .any(|d| matches!(&d.object, DeviceObject::AddressSpace(space) if space.holds(handle)));
        let (used_bytes, used_spans) = match &self.devices[at].object {
            DeviceObject::NvMap => self.nvmap.imported_usage(),
            DeviceObject::AddressSpace(_) => self
                .devices
                .iter()
                .filter_map(|d| match &d.object {
                    DeviceObject::AddressSpace(s) => Some(s.usage()),
                    _ => None,
                })
                .fold((0, 0), |(b, s), (bytes, spans)| (b + bytes, s + spans)),
        };
        let budget = ImportBudget {
            bytes: IMPORT_BYTES - used_bytes,
            spans: IMPORT_SPANS - used_spans,
        };
        let result = match &mut self.devices[at].object {
            DeviceObject::NvMap => self.nvmap.ioctl(fd, code, &bytes, budget, |a, s, p| {
                context.pin(a, s, p).ok()
            }),
            DeviceObject::AddressSpace(space) => space.ioctl(code, &bytes, budget, |handle| {
                self.nvmap.imported_view(handle)
            }),
        };
        match result {
            Err(error) => Ok(reply(&[error])),
            Ok(mut bytes) => {
                if code == 0xc0180105 && retained {
                    bytes[8..16].fill(0);
                }
                let mut response = reply(&[0]);
                if let Some(view) = output {
                    let ranges = view.backing_ranges().collect();
                    response.effect = Some(Box::new(BufferWrite {
                        view,
                        bytes,
                        ranges,
                    }));
                }
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
