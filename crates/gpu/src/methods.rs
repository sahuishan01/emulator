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
    pub clear_launches: usize,
    pub cleared_bytes: usize,
    pub draw_launches: usize,
    pub drawn_bytes: usize,
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
    UnsupportedColorState,
    UnsupportedDrawState,
    Shader(String),
    DrawInputFeedback,
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
    channels: [Option<EngineState>; 8],
}
#[derive(Clone, Debug)]
enum EngineState {
    Dma(DmaRegisters),
    Color(crate::clear::ColorRegisters, Box<crate::draw::Registers>),
}
#[derive(Debug)]
enum CopyInput {
    Prepared,
    Memory(GpuMemoryRange),
    Solid {
        color: [u8; 4],
        mask: u32,
        previous: Option<GpuMemoryRange>,
    },
}
#[derive(Debug)]
struct Copy {
    source: CopyInput,
    target: GpuMemoryRange,
    buffer: Vec<u8>,
}
#[derive(Debug)]
pub(crate) struct PreparedBatch {
    state: MethodState,
    copies: Vec<Copy>,
    clears: Vec<crate::ColorClear>,
    pub(crate) graphics_order: Vec<bool>, // false=clear, true=draw
    draws: Vec<crate::Draw>,
    draw_inputs: Vec<GpuMemoryRange>,
    snapshot_bytes: usize,
    stats: ExecutionStats,
}
impl PreparedBatch {
    pub(crate) fn next_state(&self) -> MethodState {
        self.state.clone()
    }
    pub(crate) fn clears(&self) -> &[crate::ColorClear] {
        &self.clears
    }
    pub(crate) fn draws(&self) -> &[crate::Draw] {
        &self.draws
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
            match &copy.source {
                CopyInput::Prepared => {}
                CopyInput::Memory(source) => {
                    source.read(&mut copy.buffer).expect("prepared read pin")
                }
                CopyInput::Solid {
                    color,
                    mask,
                    previous,
                } => {
                    if let Some(source) = previous {
                        source.read(&mut copy.buffer).expect("prepared color read");
                    }
                    for pixel in copy.buffer.chunks_exact_mut(4) {
                        for component in 0..4 {
                            if mask & (1 << component) != 0 {
                                pixel[component] = color[component];
                            }
                        }
                    }
                }
            }
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
            clears: Vec::new(),
            graphics_order: Vec::new(),
            draws: Vec::new(),
            draw_inputs: Vec::new(),
            snapshot_bytes: 0,
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
                let engine = match write.data {
                    MAXWELL_DMA_COPY_A => EngineState::Dma(DmaRegisters::default()),
                    crate::MAXWELL_B => {
                        EngineState::Color(crate::clear::ColorRegisters::default(), Box::default())
                    }
                    _ => return Err(ExecutionError::UnsupportedClass(write.data)),
                };
                // This subset binds a class directly. NV object handles and
                // engine selection bits require the future NV object registry.
                *channel = Some(engine);
                continue;
            }
            let engine = channel
                .as_mut()
                .ok_or(ExecutionError::UnboundSubchannel(write.subchannel))?;
            if let EngineState::Color(registers, draw_registers) = engine {
                if let Some(command) = draw_registers.write(offset, write.data)? {
                    if let Some(command) = command {
                        let (draw, pins) = draw_registers.plan(space, registers, command)?;
                        batch.add_draw(space, draw, pins, limits, &mut spans)?;
                    }
                    continue;
                }
                let clear = registers
                    .write(offset, write.data)
                    .map_err(|error| match error {
                        ExecutionError::UnsupportedMethod { offset, .. } => {
                            ExecutionError::UnsupportedMethod {
                                subchannel: write.subchannel,
                                offset,
                            }
                        }
                        error => error,
                    })?;
                if let Some(clear) = clear {
                    batch.add_clear(space, clear, limits, &mut spans)?;
                }
                continue;
            }
            let EngineState::Dma(registers) = engine else {
                unreachable!()
            };
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
                    if batch.stats.copy_launches
                        + batch.stats.clear_launches
                        + batch.stats.draw_launches
                        >= limits.launches
                        || length
                            > limits.bytes.saturating_sub(
                                batch.stats.copied_bytes
                                    + batch.stats.cleared_bytes
                                    + batch.stats.drawn_bytes
                                    + batch.snapshot_bytes,
                            )
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
                        source: CopyInput::Memory(source),
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
        if batch
            .draw_inputs
            .iter()
            .flat_map(|p| p.backing_ranges())
            .any(|a| batch.write_ranges().any(|b| overlaps(a, b)))
        {
            return Err(ExecutionError::DrawInputFeedback);
        }
        Ok(batch)
    }
}

impl PreparedBatch {
    fn add_clear(
        &mut self,
        space: &GpuAddressSpace,
        clear: crate::ColorClear,
        limits: ExecutionLimits,
        spans: &mut usize,
    ) -> Result<(), ExecutionError> {
        let length = (clear.rect[2] - clear.rect[0]) as usize * 4;
        let rows = (clear.rect[3] - clear.rect[1]) as usize;
        let bytes = length.checked_mul(rows).ok_or(ExecutionError::Capacity)?;
        if self.stats.copy_launches + self.stats.clear_launches + self.stats.draw_launches
            >= limits.launches
            || bytes
                > limits.bytes.saturating_sub(
                    self.stats.copied_bytes
                        + self.stats.cleared_bytes
                        + self.stats.drawn_bytes
                        + self.snapshot_bytes,
                )
        {
            return Err(ExecutionError::Capacity);
        }
        self.clears
            .try_reserve(1)
            .map_err(|_| ExecutionError::Capacity)?;
        let (color, mask) = clear.storage_color();
        if length > 0 {
            self.copies
                .try_reserve(rows)
                .map_err(|_| ExecutionError::Capacity)?;
            for y in clear.rect[1]..clear.rect[3] {
                let address = clear
                    .address
                    .0
                    .checked_add(
                        u64::from(y) * u64::from(clear.pitch) + u64::from(clear.rect[0]) * 4,
                    )
                    .ok_or(ExecutionError::Capacity)?;
                let target = space
                    .pin(GpuVa(address), length, Access::Write)
                    .map_err(ExecutionError::Memory)?;
                let previous = if mask != 15 {
                    Some(
                        space
                            .pin(GpuVa(address), length, Access::Read)
                            .map_err(ExecutionError::Memory)?,
                    )
                } else {
                    None
                };
                *spans += target.backing_ranges().count()
                    + previous.as_ref().map_or(0, |p| p.backing_ranges().count());
                if *spans > crate::address_space::MAX_BACKING_SPANS {
                    return Err(ExecutionError::Capacity);
                }
                let mut buffer = Vec::new();
                buffer
                    .try_reserve_exact(length)
                    .map_err(|_| ExecutionError::Capacity)?;
                buffer.resize(length, 0);
                self.copies.push(Copy {
                    source: CopyInput::Solid {
                        color,
                        mask,
                        previous,
                    },
                    target,
                    buffer,
                });
            }
        }
        self.graphics_order.push(false);
        self.clears.push(clear);
        self.stats.clear_launches += 1;
        self.stats.cleared_bytes += bytes;
        Ok(())
    }
}

fn overlaps(a: BackingRange, b: BackingRange) -> bool {
    a.id == b.id && a.offset < b.offset + b.length && b.offset < a.offset + a.length
}
impl PreparedBatch {
    fn add_draw(
        &mut self,
        space: &GpuAddressSpace,
        draw: crate::Draw,
        pins: Vec<GpuMemoryRange>,
        limits: ExecutionLimits,
        spans: &mut usize,
    ) -> Result<(), ExecutionError> {
        let pixels = draw.raster()?;
        let s = draw.surface;
        let bytes = pixels.iter().filter(|p| p.is_some()).count() * 4;
        let snapshots = pins.iter().map(GpuMemoryRange::len).sum::<usize>()
            + draw.texture.as_ref().map_or(0, |t| t.pixels.len())
            + draw.indices.as_ref().map_or(0, |i| i.values.len() * 4)
            + (draw.vertex_spirv.len() + draw.fragment_spirv.len()) * 4;
        if self.stats.copy_launches + self.stats.clear_launches + self.stats.draw_launches
            >= limits.launches
            || bytes + snapshots
                > limits.bytes.saturating_sub(
                    self.stats.copied_bytes
                        + self.stats.cleared_bytes
                        + self.stats.drawn_bytes
                        + self.snapshot_bytes,
                )
        {
            return Err(ExecutionError::Capacity);
        }
        *spans += pins
            .iter()
            .map(|p| p.backing_ranges().count())
            .sum::<usize>();
        if *spans > crate::address_space::MAX_BACKING_SPANS {
            return Err(ExecutionError::Capacity);
        }
        for y in 0..s.height {
            let mut x = 0;
            while x < s.width {
                if pixels[(y * s.width + x) as usize].is_none() {
                    x += 1;
                    continue;
                }
                let first = x;
                while x < s.width && pixels[(y * s.width + x) as usize].is_some() {
                    x += 1;
                }
                let length = ((x - first) * 4) as usize;
                let address =
                    GpuVa(s.address.0 + u64::from(y) * u64::from(s.pitch) + u64::from(first) * 4);
                let target = space
                    .pin(address, length, Access::Write)
                    .map_err(ExecutionError::Memory)?;
                *spans += target.backing_ranges().count();
                if *spans > crate::address_space::MAX_BACKING_SPANS {
                    return Err(ExecutionError::Capacity);
                }
                self.copies.push(Copy {
                    source: CopyInput::Prepared,
                    target,
                    buffer: (first..x)
                        .flat_map(|x| pixels[(y * s.width + x) as usize].expect("covered pixel"))
                        .collect(),
                });
            }
        }
        self.draw_inputs.extend(pins);
        self.snapshot_bytes += snapshots;
        self.graphics_order.push(true);
        self.draws.push(draw);
        self.stats.draw_launches += 1;
        self.stats.drawn_bytes += bytes;
        Ok(())
    }
}
