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
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// `_IOWR('G', 0x13, {u64 buffer})` VSMS_MAPPING: one {u8 gpc, u8 tpc} per
/// SM, returned in the Ioctl3 inline buffer. GM20B: one SM in each of TPC 0/1.
const VSMS_MAPPING: u32 = 0xc0084713;
const VSM_MAPPING: [u8; 4] = [0, 0, 0, 1];

/// TSG ioctls (`'T'`): bind/unbind a channel fd, enable/disable/preempt the
/// group, runlist interleave level {u32 level, u32 reserved} and timeslice
/// {u32 microseconds, u32 reserved}.
const TSG_BIND_CHANNEL: u32 = 0x40045401;
const TSG_UNBIND_CHANNEL: u32 = 0x40045402;
const TSG_ENABLE: u32 = 0x5403;
const TSG_DISABLE: u32 = 0x5404;
const TSG_PREEMPT: u32 = 0x5405;
const TSG_SET_RUNLIST_INTERLEAVE: u32 = 0x40085408;
const TSG_SET_TIMESLICE: u32 = 0x40085409;
/// Most channels one group holds.
const TSG_CHANNELS: usize = 32;

/// Scheduling state of a timeslice group. Channels run synchronously at
/// submit time, so the group only records membership and settings.
#[derive(Clone, Debug, Default)]
struct TimesliceGroup {
    channels: Vec<u32>,
    enabled: bool,
    interleave: u32,
    timeslice_us: u32,
}

/// nvhost-ctrl-gpu `_IOR('G', 0x14, u64)`: GPU timestamp in nanoseconds.
const GET_GPU_TIME: u32 = 0x80084714;

/// Distinct failure kinds kept by [`nv_failures`].
const FAILURE_KINDS: usize = 256;

/// Count and the argument words of the first occurrence.
type Failure = (u64, Vec<u32>);

thread_local! {
    static FAILURES: RefCell<BTreeMap<(String, u32, u32), Failure>> = RefCell::default();
}

fn record_failure(what: String, code: u32, error: u32, arguments: &[u8]) {
    FAILURES.with(|failures| {
        let mut failures = failures.borrow_mut();
        let key = (what, code, error);
        if failures.len() < FAILURE_KINDS || failures.contains_key(&key) {
            let entry = failures.entry(key).or_insert_with(|| {
                let words = arguments
                    .chunks_exact(4)
                    .take(16)
                    .map(|w| u32::from_le_bytes(w.try_into().expect("4 bytes")))
                    .collect();
                (0, words)
            });
            entry.0 += 1;
        }
    });
}

thread_local! {
    static SUBMIT_ERRORS: RefCell<BTreeMap<String, (u64, String)>> = RefCell::default();
}

/// Keep the GPU-side reason a GPFIFO submit was rejected, with the entry
/// count, a census of GP entry control bits and the first entries of its
/// first occurrence (diagnostics).
pub(crate) fn record_submit_error(error: String, entries: &[u8]) {
    SUBMIT_ERRORS.with(|errors| {
        let mut errors = errors.borrow_mut();
        if errors.len() < FAILURE_KINDS || errors.contains_key(&error) {
            let entry = errors.entry(error).or_insert_with(|| (0, describe_entries(entries)));
            entry.0 += 1;
        }
    });
}

fn describe_entries(entries: &[u8]) -> String {
    let words: Vec<u32> = entries
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes(w.try_into().expect("4 bytes")))
        .collect();
    let pairs: Vec<(u32, u32)> = words.chunks_exact(2).map(|p| (p[0], p[1])).collect();
    let count = |f: &dyn Fn(&(u32, u32)) -> bool| pairs.iter().filter(|p| f(p)).count();
    let first: Vec<String> = pairs
        .iter()
        .take(4)
        .map(|(low, high)| format!("{low:08x}:{high:08x}"))
        .collect();
    format!(
        "entries={} words={} zero_len={} sync={} subroutine={} priv={} conditional={} first=[{}]",
        pairs.len(),
        pairs.iter().map(|(_, high)| u64::from((high >> 10) & 0x1f_ffff)).sum::<u64>(),
        count(&|(_, high)| (high >> 10) & 0x1f_ffff == 0),
        count(&|(_, high)| high & 0x8000_0000 != 0),
        count(&|(_, high)| high & 0x200 != 0),
        count(&|(_, high)| high & 0x100 != 0),
        count(&|(low, _)| low & 3 != 0),
        first.join(" ")
    )
}

/// Distinct GPU errors behind rejected GPFIFO submits on this thread:
/// (error, (count, first occurrence's entries)). Diagnostics only.
pub fn nv_submit_errors() -> Vec<(String, (u64, String))> {
    SUBMIT_ERRORS.with(|errors| errors.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// nvdrv requests on this thread that returned a non-zero nv error:
/// ((device or "open <path>", ioctl code, error), (count, first arguments)).
/// Diagnostics only.
pub fn nv_failures() -> Vec<((String, u32, u32), Failure)> {
    FAILURES.with(|failures| failures.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

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
    /// `/dev/nvhost-tsg-gpu`: a timeslice group of channels.
    Tsg(TimesliceGroup),
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
                DeviceObject::Control | DeviceObject::Tsg(_) => {}
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
            b"/dev/nvhost-tsg-gpu" => 4,
            _ => {
                record_failure(format!("open {}", String::from_utf8_lossy(&bytes)), 0, NOT_SUPPORTED, &[]);
                return Ok(reply(&[0, NOT_SUPPORTED]));
            }
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
            4 => DeviceObject::Tsg(TimesliceGroup::default()),
            _ => DeviceObject::Control,
        };
        self.devices.push(Device { fd, object });
        self.next_fd = next;
        Ok(reply(&[fd, 0]))
    }
    /// SET_ERROR_NOTIFIER: the notifier must lie inside allocated nvmap
    /// memory. Size 0 clears it.
    fn set_error_notifier(&mut self, at: usize, input: &[u8]) -> Result<Vec<u8>, u32> {
        let wide = |i: usize| u64::from_le_bytes(input[i..i + 8].try_into().expect("validated"));
        let (offset, size, handle) = (wide(0), wide(8), nvchannel::word(input, 16));
        let notifier = if size == 0 {
            None
        } else {
            let view = self.nvmap.imported_view(handle)?;
            let end = offset.checked_add(size).ok_or(BAD_PARAMETER)?;
            if end > view.len() as u64 {
                return Err(BAD_PARAMETER);
            }
            Some((handle, offset, size))
        };
        let DeviceObject::Channel(channel) = &mut self.devices[at].object else {
            unreachable!("channel device")
        };
        channel.error_notifier = notifier;
        Ok(input.to_vec())
    }
    fn timeslice_group(&mut self, at: usize, code: u32, input: &[u8]) -> Result<Vec<u8>, u32> {
        let argument = || nvchannel::word(input, 0);
        // A channel may belong to one group at a time.
        let bound_elsewhere = |devices: &[Device], fd: u32| {
            devices.iter().enumerate().any(|(index, d)| {
                index != at && matches!(&d.object, DeviceObject::Tsg(g) if g.channels.contains(&fd))
            })
        };
        let is_channel = |devices: &[Device], fd: u32| {
            devices.iter().any(|d| d.fd == fd && matches!(d.object, DeviceObject::Channel(_)))
        };
        if code == TSG_BIND_CHANNEL && (!is_channel(&self.devices, argument()) || bound_elsewhere(&self.devices, argument())) {
            return Err(BAD_PARAMETER);
        }
        let DeviceObject::Tsg(group) = &mut self.devices[at].object else {
            unreachable!("timeslice group device")
        };
        match code {
            TSG_BIND_CHANNEL => {
                let fd = argument();
                if group.channels.contains(&fd) {
                    return Err(BAD_PARAMETER);
                }
                if group.channels.len() >= TSG_CHANNELS {
                    return Err(NO_MEMORY);
                }
                group.channels.push(fd);
            }
            TSG_UNBIND_CHANNEL => {
                let fd = argument();
                let index = group.channels.iter().position(|&c| c == fd).ok_or(BAD_PARAMETER)?;
                group.channels.remove(index);
            }
            TSG_ENABLE => group.enabled = true,
            TSG_DISABLE => group.enabled = false,
            // Work never stays queued: submissions complete synchronously.
            TSG_PREEMPT => {}
            TSG_SET_RUNLIST_INTERLEAVE => {
                if argument() > 2 || nvchannel::word(input, 4) != 0 {
                    return Err(BAD_PARAMETER);
                }
                group.interleave = argument();
            }
            TSG_SET_TIMESLICE => {
                if argument() == 0 || nvchannel::word(input, 4) != 0 {
                    return Err(BAD_PARAMETER);
                }
                group.timeslice_us = argument();
            }
            _ => return Err(NOT_SUPPORTED),
        }
        Ok(input.to_vec())
    }
    fn device_name(&self, fd: u32) -> &'static str {
        match self.devices.iter().find(|d| d.fd == fd).map(|d| &d.object) {
            Some(DeviceObject::NvMap) => "nvmap",
            Some(DeviceObject::AddressSpace) => "nvhost-as-gpu",
            Some(DeviceObject::Channel(_)) => "nvhost-gpu",
            Some(DeviceObject::Control) => "nvhost-ctrl",
            Some(DeviceObject::Tsg(_)) => "nvhost-tsg-gpu",
            None => "unknown fd",
        }
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
        let fd = self.devices.remove(at).fd;
        for device in &mut self.devices {
            if let DeviceObject::Tsg(group) = &mut device.object {
                group.channels.retain(|&channel| channel != fd);
            }
        }
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
    fn control(&mut self, code: u32, input: &[u8], time_ns: u64) -> Result<Vec<u8>, u32> {
        let mut output = input.to_vec();
        match code {
            // GET_GPU_TIME -> u64 nanoseconds on the guest's virtual clock
            // (the clock behind svcGetSystemTick).
            GET_GPU_TIME => output[0..8].copy_from_slice(&time_ns.to_le_bytes()),
            // The mapping itself goes to the inline buffer (see `ioctl`); the
            // argument comes back unchanged.
            VSMS_MAPPING => {}
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
        nvchannel::check_submit_flags(nvchannel::word(input, 12), nvchannel::word(input, 20))?;
        if nvchannel::word(input, 12) & nvchannel::SUBMIT_FENCE_WAIT != 0 {
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
        let (next, mut bytes, work, value) = channel.submit(input, space, self.syncpoints[point])?;
        // Entries passed inline (Ioctl2) are not echoed: the reply is the
        // argument buffer only.
        bytes.truncate(output.len());
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
            DeviceObject::Control => matches!(code, GET_CHARACTERISTICS | GET_TPC_MASKS_INLINE | VSMS_MAPPING),
            DeviceObject::AddressSpace => code == GET_VA_REGIONS,
            _ => false,
        };
        // Ioctl2 GPFIFO submits carry their entries in the inline input.
        let inline_entries = inline == Inline::Input
            && inline_len != 0
            && matches!(self.devices[at].object, DeviceObject::Channel(_))
            && nvchannel::submit_size(code) == Some(nvchannel::SUBMIT_HEADER_SIZE);
        let inline_output = if inline_len == 0 || inline_entries {
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
            DeviceObject::Tsg(_) => match code {
                TSG_BIND_CHANNEL | TSG_UNBIND_CHANNEL => Some(4),
                TSG_ENABLE | TSG_DISABLE | TSG_PREEMPT => Some(0),
                TSG_SET_RUNLIST_INTERLEAVE | TSG_SET_TIMESLICE => Some(8),
                _ => None,
            },
            DeviceObject::Control => match code {
                SYNC_READ => Some(8),
                SYNC_WAIT => Some(16),
                SYNC_INCR | 0x80040015 => Some(4),
                ZCULL_GET_CTX_SIZE => Some(4),
                GET_GPU_TIME | VSMS_MAPPING => Some(8),
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
            if inline_entries {
                // Exactly the announced entries; Channel::submit takes
                // entries that follow the header instead of the GPFIFO VA.
                let count = nvchannel::word(&bytes, 8) as usize;
                if count.checked_mul(8) != Some(inline_len) {
                    return Ok(reply(&[BAD_PARAMETER]));
                }
                let view = c.auto_send(1)?.expect("non-empty inline input");
                let mut entries = vec![0; inline_len];
                view.read(0, &mut entries).map_err(ServiceError::Memory)?;
                bytes.extend_from_slice(&entries);
            }
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
            DeviceObject::Control => self.control(code, &bytes, c.virtual_time_ns()),
            DeviceObject::Tsg(_) => self.timeslice_group(at, code, &bytes),
            DeviceObject::Channel(channel) => {
                if code == SET_NVMAP {
                    self.set_nvmap(at, &bytes)
                } else if code == nvchannel::SET_ERROR_NOTIFIER {
                    self.set_error_notifier(at, &bytes)
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
                    // The inline copy is the payload after the 16-byte
                    // header, or the SM mapping for VSMS_MAPPING.
                    let payload = if code == VSMS_MAPPING { &VSM_MAPPING[..] } else { &bytes[16..] };
                    let len = view.len().min(payload.len());
                    if len > 0 {
                        let view = view.slice(0, len).map_err(ServiceError::Memory)?;
                        writes.push(BufferWrite::new(view, payload[..len].to_vec()));
                    }
                } else if code == VSMS_MAPPING {
                    // Writing through the guest pointer is not implemented.
                    return Ok(reply(&[NOT_SUPPORTED]));
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
/// `nvgpu_gpu_characteristics` in the Switch's layout (which differs from
/// later Linux nvgpu: gpc_mask, SM arch, VA bits and a reserved word precede
/// the flags at 0x50), with the values a Switch reports for its GM20B.
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
    put32(0x24, 0x20); // bus_type: AXI
    put32(0x28, 0x20000); // big_page_size
    put32(0x2c, 0x20000); // compression_page_size
    put32(0x30, 27); // pde_coverage_bit_count
    put32(0x34, 0x30000); // available_big_page_sizes: 64K | 128K
    put32(0x38, 1); // gpc_mask
    put32(0x3c, 0x503); // sm_arch_sm_version (SM 5.3)
    put32(0x40, 0x503); // sm_arch_spa_version
    put32(0x44, 0x80); // sm_arch_warp_count
    put32(0x48, 0x28); // gpu_va_bit_count
    // 0x4c reserved.
    // flags (u64): partial mappings (0), sync fence fds (2), cycle stats
    // snapshot (4), timeslice groups (6). Bit 8 stays clear: NvRm then
    // creates channels without a TSG backend.
    put32(0x50, 0x55);
    put32(0x58, 0x902d); // twod_class: FERMI_TWOD_A
    put32(0x5c, 0xb197); // threed_class: MAXWELL_B
    put32(0x60, 0xb1c0); // compute_class: MAXWELL_COMPUTE_B
    put32(0x64, 0xb06f); // gpfifo_class: MAXWELL_CHANNEL_GPFIFO_A
    put32(0x68, 0xa140); // inline_to_memory_class: KEPLER_INLINE_TO_MEMORY_B
    put32(0x6c, 0xb0b5); // dma_copy_class: MAXWELL_DMA_COPY_A
    put32(0x70, 1); // max_fbps_count
    put32(0x74, 0); // fbp_en_mask
    put32(0x78, 2); // max_ltc_per_fbp
    put32(0x7c, 1); // max_lts_per_ltc
    put32(0x80, 0); // max_tex_per_tpc
    put32(0x84, 1); // max_gpc_count
    put32(0x88, 0x21d70); // rop_l2_en_mask_0
    put32(0x8c, 0); // rop_l2_en_mask_1
    c[0x90..0x96].copy_from_slice(b"gm20b\0"); // chipname (u64)
    // 0x98: gr_compbit_store_base_hw (u64) = 0.
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
                1 | 11 | 12 => {
                    let inline = match request.command_id {
                        1 => Inline::None,
                        11 => Inline::Input,
                        _ => Inline::Output,
                    };
                    let target = (request.data.len() >= 8).then(|| (input_word(request, 0), input_word(request, 4)));
                    let device = target.map_or("?", |(fd, _)| self.device_name(fd));
                    // The argument buffer as sent (A or X descriptor, slot 0).
                    let arguments = match context.auto_send(0) {
                        Ok(Some(view)) => {
                            let mut bytes = vec![0u8; view.len().min(64)];
                            view.read(0, &mut bytes).ok().map(|_| bytes).unwrap_or_default()
                        }
                        _ => Vec::new(),
                    };
                    let response = self.ioctl(request, context, inline)?;
                    let error = response.data.get(..4).map_or(0, |w| u32::from_le_bytes(w.try_into().expect("4 bytes")));
                    if error != 0 {
                        record_failure(device.to_owned(), target.map_or(0, |(_, code)| code), error, &arguments);
                    }
                    Ok(response)
                }
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
