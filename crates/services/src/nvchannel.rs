//! Original synchronous NV channel state. No host Vulkan or asynchronous queue.
use crate::nvmap::{BAD_PARAMETER, NOT_SUPPORTED, NO_MEMORY};
use emulator_gpu::{ChannelProcessor, GpuAddressSpace, PreparedSubmission, MAXWELL_DMA_COPY_A};
pub(crate) const SET_NVMAP: u32 = 0x40044801;
pub(crate) const ALLOC_CONTEXT: u32 = 0xc0104809;
pub(crate) const ALLOC_FIFO: u32 = 0xc020481a;
pub(crate) const SYNC_READ: u32 = 0xc0080014;
pub(crate) const SYNC_WAIT: u32 = 0xc0100019;
pub(crate) fn word(input: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(
        input[at..at + 4]
            .try_into()
            .expect("validated channel word"),
    )
}
pub(crate) fn submit_size(code: u32) -> Option<usize> {
    if code & 0xc000ffff == 0xc0004808 {
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
    context: bool,
    processor: ChannelProcessor,
}
impl Channel {
    pub fn new(syncpoint: u32) -> Self {
        Self {
            nvmap: None,
            space: None,
            syncpoint,
            capacity: None,
            context: false,
            processor: ChannelProcessor::default(),
        }
    }
    pub fn argument_size(code: u32) -> Option<usize> {
        match code {
            SET_NVMAP => Some(4),
            ALLOC_CONTEXT => Some(16),
            ALLOC_FIFO => Some(32),
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
                if self.context {
                    return Err(8);
                }
                if word(input, 0) != MAXWELL_DMA_COPY_A || word(input, 4) != 0 {
                    return Err(NOT_SUPPORTED);
                }
                self.context = true;
                output[8..16].copy_from_slice(&u64::from(self.syncpoint).to_le_bytes());
            }
            ALLOC_FIFO => {
                if self.nvmap.is_none() || self.space.is_none() {
                    return Err(3);
                }
                if self.capacity.is_some() {
                    return Err(8);
                }
                let count = word(input, 0) as usize;
                if count == 0 || !count.is_power_of_two() || count > 256 {
                    return Err(BAD_PARAMETER);
                }
                if word(input, 4) != 0 || word(input, 8) > 1 || input[20..].iter().any(|b| *b != 0)
                {
                    return Err(NOT_SUPPORTED);
                }
                self.capacity = Some(count);
                output[12..16].copy_from_slice(&self.syncpoint.to_le_bytes());
                output[16..20].copy_from_slice(&completed.to_le_bytes());
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
        if !self.context {
            return Err(8);
        }
        let capacity = self.capacity.ok_or(3_u32)?;
        if input.len() < 24 {
            return Err(BAD_PARAMETER);
        }
        let count = word(input, 8) as usize;
        if count == 0 || count > capacity || input.len() != 24 + count * 8 {
            return Err(BAD_PARAMETER);
        }
        // Only fence_get and already-satisfied fence_wait are modeled by caller.
        if ![2, 3].contains(&word(input, 12)) {
            return Err(NOT_SUPPORTED);
        }
        let next_value = completed.checked_add(1).ok_or(NO_MEMORY)?;
        let (processor, work) = self
            .processor
            .prepare(space, &input[24..])
            .map_err(|_| BAD_PARAMETER)?;
        let mut next = self.clone();
        next.processor = processor;
        let mut output = input.to_vec();
        output[16..20].copy_from_slice(&self.syncpoint.to_le_bytes());
        output[20..24].copy_from_slice(&next_value.to_le_bytes());
        Ok((next, output, work, next_value))
    }
}
