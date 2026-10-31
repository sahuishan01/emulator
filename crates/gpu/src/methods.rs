//! Original synchronous DMA reference executor. Register addresses/fields follow
//! NVIDIA's public MAXWELL_DMA_COPY_A interface; no vendor implementation is used.
use crate::{GpuAddressSpace, GpuMemoryError, GpuMemoryRange, GpuVa, MethodWrite};
use emulator_memory::{Access, BackingRange};

pub const MAXWELL_DMA_COPY_A: u32 = 0xb0b5;
/// NON_PIPELINED transfer, pitch source/destination, virtual addresses, one line.
pub const LINEAR_DMA_LAUNCH: u32 = 0x182;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_LAUNCHES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionLimits {
    pub bytes: usize,
    pub launches: usize,
}
impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            bytes: MAX_BYTES,
            launches: MAX_LAUNCHES,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutionStats {
    pub methods: usize,
    pub copy_launches: usize,
    pub copied_bytes: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionError {
    InvalidLimits,
    WrongAddressSpace,
    NotFront,
    UnboundSubchannel(u8),
    UnsupportedClass(u32),
    UnsupportedMethod { subchannel: u8, offset: u32 },
    InvalidAddressUpper(u32),
    UnsupportedLaunch(u32),
    UninitializedRegister,
    EmptyCopy,
    Capacity,
    OverlappingCopy,
    Memory(GpuMemoryError),
}
impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GPU method execution error: {self:?}")
    }
}
impl std::error::Error for ExecutionError {}

#[derive(Clone, Copy, Debug, Default)]
struct DmaRegisters {
    source_upper: Option<u32>,
    source_lower: Option<u32>,
    target_upper: Option<u32>,
    target_lower: Option<u32>,
    length: Option<u32>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct MethodState {
    channels: [Option<DmaRegisters>; 8],
}
#[derive(Debug)]
struct Copy {
    source: GpuMemoryRange,
    target: GpuMemoryRange,
    buffer: Vec<u8>,
}
#[derive(Debug)]
pub(crate) struct PreparedBatch {
    state: MethodState,
    copies: Vec<Copy>,
    stats: ExecutionStats,
}
impl PreparedBatch {
    pub(crate) fn next_state(&self) -> MethodState {
        self.state.clone()
    }
    pub(crate) fn stats(&self) -> ExecutionStats {
        self.stats
    }
    pub(crate) fn write_ranges(&self) -> impl Iterator<Item = BackingRange> + '_ {
        self.copies
            .iter()
            .flat_map(|copy| copy.target.backing_ranges())
    }
    /// All fallible validation/allocation finishes before any memory changes.
    /// Pins hold immutable permissions/lengths; the executor is single threaded,
    /// and no callbacks or borrowed storage escape. Exact-sized accesses below
    /// therefore cannot fail unless an internal pin invariant is broken.
    pub(crate) fn commit(mut self) -> (MethodState, ExecutionStats) {
        for copy in &mut self.copies {
            // Read at execution time so a later launch sees earlier writes,
            // including through a different virtual address of the same backing.
            copy.source
                .read(&mut copy.buffer)
                .expect("prepared read pin");
            copy.target.write(&copy.buffer).expect("prepared write pin");
        }
        (self.state, self.stats)
    }
}
impl MethodState {
    pub(crate) fn prepare(
        &self,
        space: &GpuAddressSpace,
        writes: &[MethodWrite],
        limits: ExecutionLimits,
    ) -> Result<PreparedBatch, ExecutionError> {
        if limits.bytes == 0
            || limits.bytes > MAX_BYTES
            || limits.launches == 0
            || limits.launches > MAX_LAUNCHES
        {
            return Err(ExecutionError::InvalidLimits);
        }
        let mut batch = PreparedBatch {
            state: self.clone(),
            copies: Vec::new(),
            stats: ExecutionStats {
                methods: writes.len(),
                ..ExecutionStats::default()
            },
        };
        let mut spans = 0;
        for write in writes {
            let channel = &mut batch.state.channels[usize::from(write.subchannel)];
            let offset = write.method_byte_offset();
            if offset == 0 {
                if write.data != MAXWELL_DMA_COPY_A {
                    return Err(ExecutionError::UnsupportedClass(write.data));
                }
                // This subset binds a class directly. NV object handles and
                // engine selection bits require the future NV object registry.
                *channel = Some(DmaRegisters::default());
                continue;
            }
            let registers = channel
                .as_mut()
                .ok_or(ExecutionError::UnboundSubchannel(write.subchannel))?;
            match offset {
                0x100 => {} // Class NOP carries no effect.
                0x400 | 0x408 => {
                    if write.data > 0xff {
                        return Err(ExecutionError::InvalidAddressUpper(write.data));
                    }
                    if offset == 0x400 {
                        registers.source_upper = Some(write.data);
                    } else {
                        registers.target_upper = Some(write.data);
                    }
                }
                0x404 => registers.source_lower = Some(write.data),
                0x40c => registers.target_lower = Some(write.data),
                0x418 => registers.length = Some(write.data),
                0x300 => {
                    if write.data != LINEAR_DMA_LAUNCH {
                        return Err(ExecutionError::UnsupportedLaunch(write.data));
                    }
                    let initialized =
                        |v: Option<u32>| v.ok_or(ExecutionError::UninitializedRegister);
                    let source = GpuVa(
                        (u64::from(initialized(registers.source_upper)?) << 32)
                            | u64::from(initialized(registers.source_lower)?),
                    );
                    let target = GpuVa(
                        (u64::from(initialized(registers.target_upper)?) << 32)
                            | u64::from(initialized(registers.target_lower)?),
                    );
                    let length = initialized(registers.length)? as usize;
                    if length == 0 {
                        return Err(ExecutionError::EmptyCopy);
                    }
                    if batch.stats.copy_launches >= limits.launches
                        || length > limits.bytes.saturating_sub(batch.stats.copied_bytes)
                    {
                        return Err(ExecutionError::Capacity);
                    }
                    let source = space
                        .pin(source, length, Access::Read)
                        .map_err(ExecutionError::Memory)?;
                    let target = space
                        .pin(target, length, Access::Write)
                        .map_err(ExecutionError::Memory)?;
                    spans += source.backing_ranges().count() + target.backing_ranges().count();
                    if spans > crate::address_space::MAX_BACKING_SPANS {
                        return Err(ExecutionError::Capacity);
                    }
                    // Compare backing identity, not just GPU VA: aliases can
                    // overlap even when their virtual addresses are disjoint.
                    if source.backing_ranges().any(|a| {
                        target.backing_ranges().any(|b| {
                            a.id == b.id
                                && a.offset < b.offset + b.length
                                && b.offset < a.offset + a.length
                        })
                    }) {
                        return Err(ExecutionError::OverlappingCopy);
                    }
                    let mut buffer = Vec::new();
                    buffer
                        .try_reserve_exact(length)
                        .map_err(|_| ExecutionError::Capacity)?;
                    buffer.resize(length, 0);
                    batch
                        .copies
                        .try_reserve(1)
                        .map_err(|_| ExecutionError::Capacity)?;
                    batch.copies.push(Copy {
                        source,
                        target,
                        buffer,
                    });
                    batch.stats.copy_launches += 1;
                    batch.stats.copied_bytes += length;
                }
                _ => {
                    return Err(ExecutionError::UnsupportedMethod {
                        subchannel: write.subchannel,
                        offset,
                    })
                }
            }
        }
        Ok(batch)
    }
}
