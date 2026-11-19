//! Original synchronous NV channel state. No host Vulkan or asynchronous queue.
use crate::nvmap::{BAD_PARAMETER, NOT_SUPPORTED, NO_MEMORY};
use emulator_gpu::{ChannelProcessor, GpuAddressSpace, PreparedSubmission, MAXWELL_DMA_COPY_A};
use emulator_memory::Access;
pub(crate) const SET_NVMAP: u32 = 0x40044801;
pub(crate) const ALLOC_CONTEXT: u32 = 0xc0104809;
pub(crate) const ALLOC_FIFO: u32 = 0xc020481a;
pub(crate) const SYNC_READ: u32 = 0xc0080014;
pub(crate) const SYNC_WAIT: u32 = 0xc0100019;
pub(crate) const SUBMIT_GPFIFO_3: u32 = 0xc000481b;
/// `{u64 gpfifo va, u32 entry count, u32 flags, u32 fence id, u32 fence value}`.
pub(crate) const SUBMIT_HEADER_SIZE: usize = 24;
/// `_IOWR('H', 12, {u64 offset, u64 size, u32 nvmap handle, u32 pad})`.
pub(crate) const SET_ERROR_NOTIFIER: u32 = 0xc018480c;
/// `_IOW('H', 0x14, u64)` / `_IOR('H', 0x15, u64)`: an opaque value the
/// client attaches to the channel and reads back.
pub(crate) const SET_USER_DATA: u32 = 0x40084714;
pub(crate) const GET_USER_DATA: u32 = 0x80084715;
/// `_IOW('H', 3, u32)`: job timeout in milliseconds. Submissions complete
/// synchronously, so it never expires.
pub(crate) const SET_TIMEOUT: u32 = 0x40044803;
/// `_IOWR('H', 11, {u64 gpu_va, u32 mode, u32 reserved})`: the z-cull context
/// buffer. Z-cull only accelerates depth rejection on hardware; the GPU model
/// does not need it, so the binding is recorded.
pub(crate) const ZCULL_BIND: u32 = 0xc010480b;
/// SUBMIT_GPFIFO flags: wait on the input fence before running.
pub(crate) const SUBMIT_FENCE_WAIT: u32 = 1 << 0;
/// Return a fence (syncpoint id, value) that completes after this submit.
pub(crate) const SUBMIT_FENCE_GET: u32 = 1 << 1;
/// Entries are in the hardware GP entry format. The legacy user format has
/// the same two words, so both decode identically.
pub(crate) const SUBMIT_HW_FORMAT: u32 = 1 << 2;
/// Skip the wait-for-idle before the driver's syncpoint increment. Work
/// completes synchronously here, so it has no effect.
pub(crate) const SUBMIT_SUPPRESS_WFI: u32 = 1 << 4;
/// Switch "increment by value": the pushbuffer itself raises the channel
/// syncpoint by the input fence value (host SYNCPOINTB increments). The
/// submit is accepted only when the pushbuffer's increments of the channel
/// syncpoint equal that value, so the returned fence is exact.
pub(crate) const SUBMIT_INCREMENT_BY_VALUE: u32 = 1 << 8;
const SUBMIT_FLAGS: u32 = SUBMIT_FENCE_WAIT
    | SUBMIT_FENCE_GET
    | SUBMIT_HW_FORMAT
    | SUBMIT_SUPPRESS_WFI
    | SUBMIT_INCREMENT_BY_VALUE;

/// Validate SUBMIT_GPFIFO flags against the input fence value.
pub(crate) fn check_submit_flags(flags: u32, fence_value: u32) -> Result<(), u32> {
    let _ = fence_value;
    if flags & !SUBMIT_FLAGS != 0 {
        return Err(NOT_SUPPORTED);
    }
    Ok(())
}
/// Largest GPFIFO a channel may allocate (entries).
pub(crate) const MAX_GPFIFO_ENTRIES: usize = 0x10000;

pub(crate) fn word(input: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(
        input[at..at + 4]
            .try_into()
            .expect("validated channel word"),
    )
}
pub(crate) fn submit_size(code: u32) -> Option<usize> {
    if (code & 0xc000ffff == 0xc0004808) || (code & 0xc000ffff == SUBMIT_GPFIFO_3) {
        Some(((code >> 16) & 0x3fff) as usize)
    } else {
        None
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Channel {
    pub nvmap: Option<u32>,
    pub space: Option<u32>,
    pub syncpoint: u32,
    pub capacity: Option<usize>,
    /// Error notifier (nvmap handle, offset, size). Channel errors are never
    /// raised by the synchronous model, so it is recorded but not written.
    pub error_notifier: Option<(u32, u64, u64)>,
    user_data: u64,
    timeout_ms: Option<u32>,
    zcull: Option<(u64, u32)>,
    context: Option<u32>,
    processor: ChannelProcessor,
}
impl Channel {
    pub fn new(syncpoint: u32) -> Self {
        Self {
            nvmap: None,
            space: None,
            syncpoint,
            capacity: None,
            error_notifier: None,
            user_data: 0,
            timeout_ms: None,
            zcull: None,
            context: None,
            processor: ChannelProcessor::default(),
        }
    }
    pub fn argument_size(code: u32) -> Option<usize> {
        match code {
            SET_NVMAP => Some(4),
            ALLOC_CONTEXT => Some(16),
            ALLOC_FIFO => Some(32),
            SET_ERROR_NOTIFIER => Some(24),
            SET_USER_DATA | GET_USER_DATA => Some(8),
            SET_TIMEOUT => Some(4),
            ZCULL_BIND => Some(16),
            _ => submit_size(code),
        }
    }
    pub fn initialize(&mut self, code: u32, input: &[u8], completed: u32) -> Result<Vec<u8>, u32> {
        let mut output = input.to_vec();
        match code {
            ALLOC_CONTEXT => {
                if self.space.is_none() {
                    return Err(3);
                }
                if self.context.is_some() {
                    return Err(8);
                }
                if ![
                    MAXWELL_DMA_COPY_A,
                    emulator_gpu::MAXWELL_B,
                    emulator_gpu::MAXWELL_COMPUTE_B,
                ]
                .contains(&word(input, 0))
                    || word(input, 4) != 0
                {
                    return Err(NOT_SUPPORTED);
                }
                self.context = Some(word(input, 0));
                output[8..16].copy_from_slice(&u64::from(self.syncpoint).to_le_bytes());
            }
            ALLOC_FIFO => {
                if self.nvmap.is_none() || self.space.is_none() {
                    return Err(3);
                }
                if self.capacity.is_some() {
                    return Err(8);
                }
                // ALLOC_GPFIFO_EX2: {u32 entries, u32 flags, u32 unknown,
                // fence out {id, value}, u32 reserved[3]}. Entries only bound
                // the GPFIFO entries one submit may carry here. Flag bit 0 and
                // the unknown word (0 or 1) have no observable effect on the
                // synchronous channel model; NVN sends flags 1.
                let count = word(input, 0) as usize;
                if count == 0 || !count.is_power_of_two() || count > MAX_GPFIFO_ENTRIES {
                    return Err(BAD_PARAMETER);
                }
                if word(input, 4) > 1 || word(input, 8) > 1 || input[20..].iter().any(|b| *b != 0) {
                    return Err(NOT_SUPPORTED);
                }
                self.capacity = Some(count);
                output[12..16].copy_from_slice(&self.syncpoint.to_le_bytes());
                output[16..20].copy_from_slice(&completed.to_le_bytes());
            }
            SET_USER_DATA => {
                self.user_data = u64::from_le_bytes(input[..8].try_into().expect("8 bytes"))
            }
            GET_USER_DATA => output.copy_from_slice(&self.user_data.to_le_bytes()),
            SET_TIMEOUT => {
                let timeout = word(input, 0);
                if timeout == 0 {
                    return Err(BAD_PARAMETER);
                }
                self.timeout_ms = Some(timeout);
            }
            ZCULL_BIND => {
                // Modes: 0 global, 1 no context switch, 2 separate buffer,
                // 3 part of the regular context buffer.
                let va = u64::from_le_bytes(input[..8].try_into().expect("8 bytes"));
                let mode = word(input, 8);
                if mode > 3 || word(input, 12) != 0 || (mode == 2 && va == 0) {
                    return Err(BAD_PARAMETER);
                }
                self.zcull = Some((va, mode));
            }
            _ => return Err(NOT_SUPPORTED),
        }
        Ok(output)
    }
    pub fn submit(
        &self,
        input: &[u8],
        space: &GpuAddressSpace,
        completed: u32,
    ) -> Result<(Self, Vec<u8>, PreparedSubmission, u32), u32> {
        if self.context.is_none() {
            return Err(8);
        }
        let capacity = self.capacity.ok_or(3_u32)?;
        if input.len() < 24 {
            return Err(BAD_PARAMETER);
        }
        let count = word(input, 8) as usize;
        if count == 0 || count > capacity {
            return Err(BAD_PARAMETER);
        }
        let flags = word(input, 12);
        check_submit_flags(flags, word(input, 20))?;

        let entries_bytes: Vec<u8> = if input.len() >= 24 + count * 8 {
            input[24..24 + count * 8].to_vec()
        } else {
            let gpfifo_ptr = u64::from_le_bytes(input[0..8].try_into().map_err(|_| BAD_PARAMETER)?);
            if gpfifo_ptr == 0 {
                return Err(BAD_PARAMETER);
            }
            let view = space
                .pin(emulator_gpu::GpuVa(gpfifo_ptr), count * 8, Access::Read)
                .map_err(|_| BAD_PARAMETER)?;
            let mut buf = vec![0u8; count * 8];
            view.read(&mut buf).map_err(|_| BAD_PARAMETER)?;
            buf
        };

        let (processor, work) = self
            .processor
            .prepare_for_class(space, &entries_bytes, self.context)
            .map_err(|error| {
                crate::nv::record_submit_error(error, &entries_bytes);
                BAD_PARAMETER
            })?;
        // Syncpoint increments from the pushbuffer: only the channel's own
        // syncpoint is modelled; with increment-by-value their count must
        // equal the fence value the driver declared.
        let increments = work.syncpoint_increments();
        if increments.iter().any(|&id| id != self.syncpoint) {
            return Err(NOT_SUPPORTED);
        }
        let own = increments.len() as u32;
        if flags & SUBMIT_INCREMENT_BY_VALUE != 0 && own != word(input, 20) {
            crate::nv::record_submit_error(
                format!(
                    "increment-by-value {} but pushbuffer increments {own}",
                    word(input, 20)
                ),
                &entries_bytes,
            );
            return Err(NOT_SUPPORTED);
        }
        let mut next_value = completed.checked_add(own).ok_or(NO_MEMORY)?;
        if flags & SUBMIT_FENCE_GET != 0 {
            next_value = next_value.checked_add(1).ok_or(NO_MEMORY)?;
        }
        let mut next = self.clone();
        next.processor = processor;
        let mut output = input.to_vec();
        output[16..20].copy_from_slice(&self.syncpoint.to_le_bytes());
        output[20..24].copy_from_slice(&next_value.to_le_bytes());
        Ok((next, output, work, next_value))
    }
}
