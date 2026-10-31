//! Original bounded NV protocol, synchronous channels and transactional effects.
use crate::nvas::{NvAddressSpace, BIND_CHANNEL};
use crate::nvchannel::{self, Channel, SET_NVMAP, SYNC_READ, SYNC_WAIT};
use crate::nvmap::{
    ImportBudget, NvMap, BAD_PARAMETER, IMPORT_BYTES, IMPORT_SPANS, NOT_SUPPORTED, NO_MEMORY,
};
use emulator_gpu::PreparedSubmission;
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
    spaces: Vec<SpaceRecord>,
    syncpoints: Vec<u32>,
}
impl Default for NvDriver {
    fn default() -> Self {
        Self {
            arena: None,
            devices: Vec::new(),
            next_fd: 1,
            nvmap: NvMap::default(),
            spaces: Vec::new(),
            syncpoints: Vec::new(),
        }
    }
}
#[derive(Clone, Debug)]
struct SpaceRecord {
    id: u32,
    state: NvAddressSpace,
}
#[derive(Clone, Debug)]
struct Device {
    fd: u32,
    object: DeviceObject,
}
#[derive(Clone, Debug)]
enum DeviceObject {
    NvMap,
    AddressSpace,
    Channel(Box<Channel>),
    Control,
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
#[derive(Debug)]
struct SubmitEffect {
    work: PreparedSubmission,
    output: BufferWrite,
    writes: Vec<BackingRange>,
}
impl PreparedServiceEffect for SubmitEffect {
    fn write_ranges(&self) -> &[BackingRange] {
        &self.writes
    }
    fn commit(self: Box<Self>) {
        let _ = self.work.commit();
        Box::new(self.output).commit();
    }
}
fn overlap(a: &BackingRange, b: &BackingRange) -> bool {
    a.id == b.id && a.offset < b.offset + b.length && b.offset < a.offset + a.length
}
fn reply(words: &[u32]) -> ServiceReply {
    ServiceReply {
        data: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
        ..ServiceReply::default()
    }
}
fn input_word(request: &Request, offset: usize) -> u32 {
    nvchannel::word(&request.data, offset)
}
impl NvDriver {
    fn space(&self, id: u32) -> &NvAddressSpace {
        &self
            .spaces
            .iter()
            .find(|s| s.id == id)
            .expect("retained address space")
            .state
    }
    fn collect_closed(&mut self) {
        let mut owners = Vec::new();
        let mut spaces = Vec::new();
        for d in &self.devices {
            match &d.object {
                DeviceObject::NvMap => owners.push(d.fd),
                DeviceObject::AddressSpace => spaces.push(d.fd),
                DeviceObject::Channel(c) => {
                    if let Some(fd) = c.nvmap {
                        owners.push(fd)
                    }
                    if let Some(id) = c.space {
                        spaces.push(id)
                    }
                }
                DeviceObject::Control => {}
            }
        }
        self.nvmap.retain_owners(&owners);
        self.spaces.retain(|s| spaces.contains(&s.id));
    }
    fn open(&mut self, r: &Request, c: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        c.require_buffers(1, 0)?;
        super::payload(r, 0)?;
        if self.arena.is_none() {
            return Ok(reply(&[0, 3]));
        }
        let path = c.send_buffer(0)?;
        if path.is_empty() || path.len() > 64 {
            return Ok(reply(&[0, BAD_PARAMETER]));
        }
        let mut bytes = vec![0; path.len()];
        path.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        if bytes.iter().any(|b| !b.is_ascii() || *b == 0) {
            return Ok(reply(&[0, BAD_PARAMETER]));
        }
        let kind = match bytes.as_slice() {
            b"/dev/nvmap" => 0,
            b"/dev/nvhost-as-gpu" => 1,
            b"/dev/nvhost-gpu" => 2,
            b"/dev/nvhost-ctrl" => 3,
            _ => return Ok(reply(&[0, NOT_SUPPORTED])),
        };
        if self.devices.len() >= 16 || (kind == 2 && self.syncpoints.len() >= 32) {
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
        let object = match kind {
            0 => DeviceObject::NvMap,
            1 => {
                self.spaces.push(SpaceRecord {
                    id: fd,
                    state: NvAddressSpace::default(),
                });
                DeviceObject::AddressSpace
            }
            2 => {
                self.syncpoints.push(0);
                DeviceObject::Channel(Box::new(Channel::new(self.syncpoints.len() as u32)))
            }
            _ => DeviceObject::Control,
        };
        self.devices.push(Device { fd, object });
        self.next_fd = next;
        Ok(reply(&[fd, 0]))
    }
    fn close(&mut self, r: &Request, c: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        c.require_buffers(0, 0)?;
        super::payload(r, 4)?;
        if self.arena.is_none() {
            return Ok(reply(&[3]));
        }
        let Some(at) = self.devices.iter().position(|d| d.fd == input_word(r, 0)) else {
            return Ok(reply(&[BAD_PARAMETER]));
        };
        self.devices.remove(at);
        self.collect_closed();
        Ok(reply(&[0]))
    }
    fn control(&self, code: u32, input: &[u8]) -> Result<Vec<u8>, u32> {
        let id = nvchannel::word(input, 0);
        let value = *self
            .syncpoints
            .get(id.checked_sub(1).ok_or(BAD_PARAMETER)? as usize)
            .ok_or(BAD_PARAMETER)?;
        let mut output = input.to_vec();
        match code {
            SYNC_READ => output[4..8].copy_from_slice(&value.to_le_bytes()),
            SYNC_WAIT => {
                if nvchannel::word(input, 8) != 0 {
                    return Err(NOT_SUPPORTED);
                }
                if value < nvchannel::word(input, 4) {
                    return Err(5);
                }
                output[12..16].copy_from_slice(&value.to_le_bytes());
            }
            _ => return Err(NOT_SUPPORTED),
        }
        Ok(output)
    }
    fn bind(&mut self, asfd: u32, input: &[u8]) -> Result<Vec<u8>, u32> {
        self.space(asfd).core()?;
        let target = nvchannel::word(input, 0);
        let Some(Device {
            object: DeviceObject::Channel(channel),
            ..
        }) = self.devices.iter_mut().find(|d| d.fd == target)
        else {
            return Err(BAD_PARAMETER);
        };
        if channel.nvmap.is_none() {
            return Err(3);
        }
        if channel.space.is_some() {
            return Err(8);
        }
        channel.space = Some(asfd);
        Ok(input.to_vec())
    }
    fn set_nvmap(&mut self, at: usize, input: &[u8]) -> Result<Vec<u8>, u32> {
        let fd = nvchannel::word(input, 0);
        if !self
            .devices
            .iter()
            .any(|d| d.fd == fd && matches!(d.object, DeviceObject::NvMap))
        {
            return Err(BAD_PARAMETER);
        }
        let DeviceObject::Channel(c) = &mut self.devices[at].object else {
            unreachable!()
        };
        if c.nvmap.is_some() {
            return Err(8);
        }
        c.nvmap = Some(fd);
        Ok(input.to_vec())
    }
    fn submit(&mut self, at: usize, input: &[u8], output: MemoryView) -> Result<ServiceReply, u32> {
        let DeviceObject::Channel(channel) = &self.devices[at].object else {
            unreachable!()
        };
        let space = self.space(channel.space.ok_or(3_u32)?).core()?;
        if input.len() < 24 {
            return Err(BAD_PARAMETER);
        }
        if ![2, 3].contains(&nvchannel::word(input, 12)) {
            return Err(NOT_SUPPORTED);
        }
        if nvchannel::word(input, 12) & 1 != 0 {
            let id = nvchannel::word(input, 16);
            let value = *self
                .syncpoints
                .get(id.checked_sub(1).ok_or(BAD_PARAMETER)? as usize)
                .ok_or(BAD_PARAMETER)?;
            if value < nvchannel::word(input, 20) {
                return Err(5);
            }
        }
        let point = channel.syncpoint as usize - 1;
        let (next, bytes, work, value) = channel.submit(input, space, self.syncpoints[point])?;
        let gpu_writes: Vec<_> = work.write_ranges().collect();
        let out_ranges: Vec<_> = output.backing_ranges().collect();
        if gpu_writes
            .iter()
            .any(|a| out_ranges.iter().any(|b| overlap(a, b)))
        {
            return Err(BAD_PARAMETER);
        }
        if gpu_writes.len() + out_ranges.len() > 4096 {
            return Err(NO_MEMORY);
        }
        let mut writes = gpu_writes;
        writes.extend(out_ranges.iter().copied());
        let effect = SubmitEffect {
            work,
            output: BufferWrite {
                view: output,
                bytes,
                ranges: out_ranges,
            },
            writes,
        };
        self.devices[at].object = DeviceObject::Channel(Box::new(next));
        self.syncpoints[point] = value;
        let mut response = reply(&[0]);
        response.effect = Some(Box::new(effect));
        Ok(response)
    }
    fn ioctl(&mut self, r: &Request, c: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        super::payload(r, 8)?;
        let fd = input_word(r, 0);
        let code = input_word(r, 4);
        let send = usize::from(code & (1 << 30) != 0);
        let receive = usize::from(code & (1 << 31) != 0);
        c.require_buffers(send, receive)?;
        if self.arena.is_none() {
            return Ok(reply(&[3]));
        }
        let Some(at) = self.devices.iter().position(|d| d.fd == fd) else {
            return Ok(reply(&[BAD_PARAMETER]));
        };
        let size = match &self.devices[at].object {
            DeviceObject::NvMap => NvMap::argument_size(code),
            DeviceObject::AddressSpace => NvAddressSpace::argument_size(code),
            DeviceObject::Channel(_) => Channel::argument_size(code),
            DeviceObject::Control => match code {
                SYNC_READ => Some(8),
                SYNC_WAIT => Some(16),
                _ => None,
            },
        };
        let Some(size) = size else {
            return Ok(reply(&[NOT_SUPPORTED]));
        };
        if size > 2072 {
            return Ok(reply(&[BAD_PARAMETER]));
        }
        let input = c.send_buffer(0)?;
        let output = if receive == 1 {
            Some(c.receive_buffer(0)?)
        } else {
            None
        };
        if input.len() != size || output.as_ref().is_some_and(|v| v.len() != size) {
            return Ok(reply(&[BAD_PARAMETER]));
        }
        let mut bytes = vec![0; size];
        input.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        if matches!(&self.devices[at].object, DeviceObject::Channel(_))
            && nvchannel::submit_size(code).is_some()
        {
            return Ok(
                match self.submit(at, &bytes, output.expect("inout submit")) {
                    Ok(reply) => reply,
                    Err(error) => reply(&[error]),
                },
            );
        }
        let handle = if code == 0xc0180105 {
            nvchannel::word(&bytes, 0)
        } else {
            0
        };
        let retained = self.spaces.iter().any(|s| s.state.holds(handle));
        let (used_bytes, used_spans) = if matches!(self.devices[at].object, DeviceObject::NvMap) {
            self.nvmap.imported_usage()
        } else {
            self.spaces
                .iter()
                .map(|s| s.state.usage())
                .fold((0, 0), |(b, s), (bytes, spans)| (b + bytes, s + spans))
        };
        let budget = ImportBudget {
            bytes: IMPORT_BYTES - used_bytes,
            spans: IMPORT_SPANS - used_spans,
        };
        let result = match &self.devices[at].object {
            DeviceObject::NvMap => self
                .nvmap
                .ioctl(fd, code, &bytes, budget, |a, s, p| c.pin(a, s, p).ok()),
            DeviceObject::AddressSpace => {
                if code == BIND_CHANNEL {
                    self.bind(fd, &bytes)
                } else {
                    self.spaces
                        .iter_mut()
                        .find(|s| s.id == fd)
                        .expect("open space")
                        .state
                        .ioctl(code, &bytes, budget, |h| self.nvmap.imported_view(h))
                }
            }
            DeviceObject::Control => self.control(code, &bytes),
            DeviceObject::Channel(channel) => {
                if code == SET_NVMAP {
                    self.set_nvmap(at, &bytes)
                } else {
                    let completed = self.syncpoints[channel.syncpoint as usize - 1];
                    let DeviceObject::Channel(channel) = &mut self.devices[at].object else {
                        unreachable!()
                    };
                    channel.initialize(code, &bytes, completed)
                }
            }
        };
        match result {
            Err(error) => Ok(reply(&[error])),
            Ok(mut bytes) => {
                if code == 0xc0180105 && retained {
                    bytes[8..16].fill(0)
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
