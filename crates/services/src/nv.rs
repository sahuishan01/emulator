//! Original bounded NV protocol, synchronous channels and transactional effects.
use crate::nvas::{NvAddressSpace, BIND_CHANNEL, GET_VA_REGIONS};
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

pub(crate) const ZCULL_GET_CTX_SIZE: u32 = 0x80044701;
/// `_IOR('G', 2, nvgpu_gpu_zcull_get_info_args)`: ten u32 out.
pub(crate) const ZCULL_GET_INFO: u32 = 0x80284702;
/// `_IOW('G', 3, nvgpu_gpu_zbc_set_table_args)`.
pub(crate) const ZBC_SET_TABLE: u32 = 0x402c4703;
/// `_IOWR('G', 4, nvgpu_gpu_zbc_query_table_args)`.
pub(crate) const ZBC_QUERY_TABLE: u32 = 0xc0344704;
/// `NVGPU_GPU_IOCTL_GET_CHARACTERISTICS`: 16-byte request header
/// (buf_size, buf_addr) followed by the 0xA0-byte characteristics.
pub(crate) const GET_CHARACTERISTICS: u32 = 0xc0b04705;
const CHARACTERISTICS_SIZE: usize = 0xa0;
/// `NVGPU_GPU_IOCTL_GET_TPC_MASKS` with the 0x18-byte header
/// {mask_buf_size, reserved[3], mask (u64)}; GM20B has one GPC with TPCs 0-1.
pub(crate) const GET_TPC_MASKS_INLINE: u32 = 0xc0184706;
const TPC_MASK: u32 = 0x3;
pub(crate) const GET_TPC_MASKS: u32 = 0x80044706;
pub(crate) const SYNC_INCR: u32 = 0xc0040015;
#[derive(Clone, Debug)]
pub struct NvDriver {
    arena: Option<Rc<TransferMemory>>,
    graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>,
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
            graphics: None,
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
use crate::BufferWrite;
#[derive(Debug)]
struct SubmitEffect {
    work: PreparedSubmission,
    output: BufferWrite,
    writes: Vec<BackingRange>,
    host: Option<(
        Rc<dyn emulator_gpu::GraphicsBackend>,
        emulator_gpu::GraphicsFrame,
    )>,
}
impl PreparedServiceEffect for SubmitEffect {
    fn complete(&mut self) -> Result<(), ServiceError> {
        if let Some((backend, frame)) = &self.host {
            backend.render(frame).map_err(ServiceError::DeviceFault)?;
        }
        Ok(())
    }
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
    pub fn with_graphics(graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>) -> Self {
        Self {
            graphics,
            ..Self::default()
        }
    }
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
            b"/dev/nvhost-gpu" | b"/dev/nvhost-channel" => 2,
            b"/dev/nvhost-ctrl" | b"/dev/nvhost-ctrl-gpu" => 3,
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
    fn query_event(
        &mut self,
        r: &Request,
        c: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        c.require_buffers(0, 0)?;
        super::payload(r, 8)?;
        let fd = input_word(r, 0);
        let _event_id = input_word(r, 4);
        let Some(_) = self.devices.iter().position(|d| d.fd == fd) else {
            return Ok(reply(&[BAD_PARAMETER]));
        };
        let mut response = reply(&[0]);
        response
            .moved_objects
            .push(Box::new(crate::event::EventHandle::new(false)));
        Ok(response)
    }
    fn control(&mut self, code: u32, input: &[u8]) -> Result<Vec<u8>, u32> {
        let mut output = input.to_vec();
        match code {
            SYNC_READ => {
                let id = nvchannel::word(input, 0);
                let value = *self
                    .syncpoints
                    .get(id.checked_sub(1).ok_or(BAD_PARAMETER)? as usize)
                    .ok_or(BAD_PARAMETER)?;
                output[4..8].copy_from_slice(&value.to_le_bytes());
            }
            SYNC_WAIT => {
                let id = nvchannel::word(input, 0);
                let value = *self
                    .syncpoints
                    .get(id.checked_sub(1).ok_or(BAD_PARAMETER)? as usize)
                    .ok_or(BAD_PARAMETER)?;
                if nvchannel::word(input, 8) != 0 {
                    return Err(NOT_SUPPORTED);
                }
                if value < nvchannel::word(input, 4) {
                    return Err(5);
                }
                output[12..16].copy_from_slice(&value.to_le_bytes());
            }
            SYNC_INCR | 0x80040015 => {
                let id = nvchannel::word(input, 0);
                let p = self
                    .syncpoints
                    .get_mut(id.checked_sub(1).ok_or(BAD_PARAMETER)? as usize)
                    .ok_or(BAD_PARAMETER)?;
                *p = p.wrapping_add(1);
            }
            GET_CHARACTERISTICS => {
                // Header: report the full struct size, keep the guest address.
                output[0..8].copy_from_slice(&(CHARACTERISTICS_SIZE as u64).to_le_bytes());
                output[16..].copy_from_slice(&gpu_characteristics());
            }
            GET_TPC_MASKS_INLINE => {
                output[0..4].copy_from_slice(&4u32.to_le_bytes());
                output[16..20].copy_from_slice(&TPC_MASK.to_le_bytes());
            }
            ZCULL_GET_CTX_SIZE => {
                if output.len() >= 4 {
                    output[0..4].copy_from_slice(&1u32.to_le_bytes());
                }
            }
            GET_TPC_MASKS | 0x800c4706 | 0xc00c4706 => {
                if output.len() >= 4 {
                    output[0..4].copy_from_slice(&0x3u32.to_le_bytes());
                }
                if output.len() >= 12 {
                    output[0..4].copy_from_slice(&4u32.to_le_bytes());
                    output[4..8].copy_from_slice(&0u32.to_le_bytes());
                    output[8..12].copy_from_slice(&0x3u32.to_le_bytes());
                }
            }
            ZCULL_GET_INFO => {
                // GM20B zcull geometry: width/height align, pixel squares per
                // aliquot, aliquot total, region byte multiplier, region and
                // subregion header sizes, subregion width/height align, count.
                let info = [0x20u32, 0x20, 0x400, 0x800, 0x20, 0x20, 0xc0, 0x20, 0x40, 0x10];
                for (i, v) in info.iter().enumerate() {
                    output[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            // ZBC clear-colour table: entries are accepted, queries report empty.
            ZBC_SET_TABLE | ZBC_QUERY_TABLE => {}
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
        // Binding may precede SET_NVMAP; submission requires both.
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
        channel.nvmap.ok_or(3_u32)?;
        let space = self.space(channel.space.ok_or(3_u32)?).core()?;
        if input.len() < 24 {
            return Err(BAD_PARAMETER);
        }
        if nvchannel::word(input, 12) > 3 {
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
        let host = if let Some(backend) = &self.graphics {
            work.graphics_frame().ok().map(|f| (Rc::clone(backend), f))
        } else {
            None
        };
        let effect = SubmitEffect {
            host,
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
    /// Ioctl (1), Ioctl2 (11, extra inline input) and Ioctl3 (12, extra
    /// inline output). No implemented ioctl consumes or produces inline data,
    /// so a non-empty inline buffer is rejected rather than ignored.
    fn ioctl(
        &mut self,
        r: &Request,
        c: &ServiceContext<'_>,
        inline: Inline,
    ) -> Result<ServiceReply, ServiceError> {
        // Auto-select receive buffers append one u16 size each (padded).
        if r.data.len() < 8 || r.data.len() > 16 {
            super::payload(r, 8)?;
        }
        let fd = input_word(r, 0);
        let code = input_word(r, 4);
        let send = usize::from(code & (1 << 30) != 0);
        let receive = usize::from(code & (1 << 31) != 0);
        // Slot 0 is the ioctl argument, slot 1 the inline buffer.
        let (want_send, want_receive) = match inline {
            Inline::None => (send, receive),
            Inline::Input => (2, receive),
            Inline::Output => (send, 2),
        };
        c.require_auto_buffers(want_send, want_receive)?;
        let inline_len = match inline {
            Inline::None => 0,
            Inline::Input => c.auto_send(1)?.map_or(0, |v| v.len()),
            Inline::Output => c.auto_receive(1)?.map_or(0, |v| v.len()),
        };
        if self.arena.is_none() {
            return Ok(reply(&[3]));
        }
        let Some(at) = self.devices.iter().position(|d| d.fd == fd) else {
            return Ok(reply(&[BAD_PARAMETER]));
        };
        // Ioctl3 forms implemented here copy the payload after a 16-byte
        // header into the inline output buffer.
        let inline_capable = match self.devices[at].object {
            DeviceObject::Control => matches!(code, GET_CHARACTERISTICS | GET_TPC_MASKS_INLINE),
            DeviceObject::AddressSpace => code == GET_VA_REGIONS,
            _ => false,
        };
        let inline_output = if inline_len == 0 {
            None
        } else if inline == Inline::Output && inline_capable {
            c.auto_receive(1)?
        } else {
            return Ok(reply(&[NOT_SUPPORTED]));
        };
        let size = match &self.devices[at].object {
            DeviceObject::NvMap => NvMap::argument_size(code),
            DeviceObject::AddressSpace => NvAddressSpace::argument_size(code),
            DeviceObject::Channel(_) => Channel::argument_size(code),
            DeviceObject::Control => match code {
                SYNC_READ => Some(8),
                SYNC_WAIT => Some(16),
                SYNC_INCR | 0x80040015 => Some(4),
                ZCULL_GET_CTX_SIZE => Some(4),
                ZCULL_GET_INFO => Some(40),
                ZBC_SET_TABLE => Some(0x2c),
                ZBC_QUERY_TABLE => Some(0x34),
                GET_CHARACTERISTICS => Some(16 + CHARACTERISTICS_SIZE),
                GET_TPC_MASKS_INLINE => Some(0x18),
                GET_TPC_MASKS | 0x800c4706 | 0xc00c4706 => {
                    Some(if code & 0x3fff_0000 == 0x0004_0000 { 4 } else { 12 })
                }
                _ => None,
            },
        };
        let Some(size) = size else {
            return Ok(reply(&[NOT_SUPPORTED]));
        };
        if size > 2072 {
            return Ok(reply(&[BAD_PARAMETER]));
        }
        let mut bytes = vec![0; size];
        if send == 1 {
            let Some(input) = c.auto_send(0)?.filter(|v| v.len() == size) else {
                return Ok(reply(&[BAD_PARAMETER]));
            };
            input.read(0, &mut bytes).map_err(ServiceError::Memory)?;
        }
        let output = if receive == 1 {
            let Some(out) = c.auto_receive(0)?.filter(|v| v.len() == size) else {
                return Ok(reply(&[BAD_PARAMETER]));
            };
            Some(out)
        } else {
            None
        };
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
                let mut writes = Vec::new();
                if let Some(view) = inline_output {
                    // The inline copy is the payload after the 16-byte header.
                    let len = view.len().min(bytes.len() - 16);
                    writes.push(BufferWrite::new(view, bytes[16..16 + len].to_vec()));
                }
                if let Some(view) = output {
                    writes.push(BufferWrite::new(view, bytes));
                }
                if !writes.is_empty() {
                    response.effect = Some(Box::new(BufferWrites::new(writes)));
                }
                Ok(response)
            }
        }
    }
}
/// Several deferred buffer writes committed together.
#[derive(Debug)]
struct BufferWrites {
    writes: Vec<BufferWrite>,
    ranges: Vec<emulator_memory::BackingRange>,
}
impl BufferWrites {
    fn new(writes: Vec<BufferWrite>) -> Self {
        let ranges = writes.iter().flat_map(|w| w.ranges.iter().cloned()).collect();
        Self { writes, ranges }
    }
}
impl emulator_kernel::service::PreparedServiceEffect for BufferWrites {
    fn write_ranges(&self) -> &[emulator_memory::BackingRange] {
        &self.ranges
    }
    fn commit(self: Box<Self>) {
        for write in self.writes {
            Box::new(write).commit();
        }
    }
}

/// `nvgpu_gpu_characteristics` for the Tegra X1 GM20B.
fn gpu_characteristics() -> [u8; CHARACTERISTICS_SIZE] {
    let mut c = [0u8; CHARACTERISTICS_SIZE];
    let mut put32 = |at: usize, v: u32| c[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put32(0x00, 0x120); // arch: GM200
    put32(0x04, 0x0b); // impl: GM20B
    put32(0x08, 0xa1); // rev
    put32(0x0c, 1); // num_gpc
    put32(0x10, 0x40000); // L2 cache size (u64)
    // 0x18: on-board video memory size (u64) = 0, unified memory.
    put32(0x20, 2); // num_tpc_per_gpc
    put32(0x24, 0x20); // bus_type: platform
    put32(0x28, 0x20000); // big_page_size
    put32(0x2c, 0x20000); // compression_page_size
    put32(0x30, 27); // pde_coverage_bit_count
    put32(0x34, 0x30000); // available_big_page_sizes
    put32(0x38, 0x7); // flags (u64): sparse, sync fence fds, cycle stats
    put32(0x40, 0x902d); // twod_class: FERMI_TWOD_A
    put32(0x44, 0xb197); // threed_class: MAXWELL_B
    put32(0x48, 0xb1c0); // compute_class: MAXWELL_COMPUTE_B
    put32(0x4c, 0xb06f); // gpfifo_class: MAXWELL_CHANNEL_GPFIFO_A
    put32(0x50, 0xa140); // inline_to_memory_class: KEPLER_INLINE_TO_MEMORY_B
    put32(0x54, 0xb0b5); // dma_copy_class: MAXWELL_DMA_COPY_A
    put32(0x58, 1); // max_fbps_count
    put32(0x5c, 0); // fbp_en_mask
    put32(0x60, 2); // max_ltc_per_fbp
    put32(0x64, 1); // max_lts_per_ltc
    put32(0x68, 0); // max_tex_per_tpc
    put32(0x6c, 1); // max_gpc_count
    put32(0x70, 0x21); // rop_l2_en_mask_0
    put32(0x74, 0); // rop_l2_en_mask_1
    c[0x78..0x7e].copy_from_slice(b"gm20b\0"); // chipname (u64)
    c
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Inline {
    None,
    Input,
    Output,
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
                1 => self.ioctl(request, context, Inline::None),
                11 => self.ioctl(request, context, Inline::Input),
                12 => self.ioctl(request, context, Inline::Output),
                2 => self.close(request, context),
                4 => self.query_event(request, context),
                // SetAruid (8), SetGraphicsFirmwareMemoryMarginEnabled (13).
                8 | 13 => Ok(reply(&[0])),
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
