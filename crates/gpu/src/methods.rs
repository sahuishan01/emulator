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
    InputsNotSnapshotted,
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
    depth: bool,
    source: CopyInput,
    target: GpuMemoryRange,
    buffer: Vec<u8>,
}
impl Copy {
    fn resolve(&mut self, earlier: &[Copy]) -> Result<(), ExecutionError> {
        match &self.source {
            CopyInput::Prepared => {}
            CopyInput::Memory(source) => read_projected(source, &mut self.buffer, earlier)?,
            CopyInput::Solid {
                color,
                mask,
                previous,
            } => {
                if let Some(source) = previous {
                    read_projected(source, &mut self.buffer, earlier)?;
                }
                for pixel in self.buffer.chunks_exact_mut(4) {
                    for component in 0..4 {
                        if mask & (1 << component) != 0 {
                            pixel[component] = color[component];
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
pub(crate) struct PreparedBatch {
    state: MethodState,
    copies: Vec<Copy>,
    clears: Vec<crate::ColorClear>,
    pub(crate) graphics_order: Vec<bool>, // false=clear, true=draw
    draws: Vec<crate::Draw>,
    draw_inputs: Vec<GpuMemoryRange>,
    depth_inputs: Vec<GpuMemoryRange>,
    snapshot_bytes: usize,
    inputs_snapshotted: bool,
    resolved_copies: usize,
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
    pub(crate) fn read_after(
        &self,
        source: &GpuMemoryRange,
        output: &mut [u8],
    ) -> Result<(), ExecutionError> {
        if !self.inputs_snapshotted {
            return Err(ExecutionError::InputsNotSnapshotted);
        }
        if output.len() > MAX_BYTES {
            return Err(ExecutionError::Capacity);
        }
        read_projected(source, output, &self.copies)
    }
    pub(crate) fn snapshot_inputs(&mut self) -> Result<(), ExecutionError> {
        {
            for index in self.resolved_copies..self.copies.len() {
                let (earlier, current) = self.copies.split_at_mut(index);
                current[0].resolve(earlier)?;
            }
            self.inputs_snapshotted = true;
            self.resolved_copies = self.copies.len();
        }
        Ok(())
    }
    pub(crate) fn verify_frame_pixels(&self, pixels: &[u8]) -> Result<(), String> {
        let clear = self.clears.first().ok_or("Missing frame clear")?;
        let row_bytes = clear.width as usize * 4;
        let rows = clear.height as usize;
        if pixels.len() != row_bytes * rows || self.copies.len() < rows {
            return Err("Frame projection size mismatch".into());
        }
        for (row, expected) in self.copies[..rows]
            .iter()
            .zip(pixels.chunks_exact(row_bytes))
        {
            // A full, unmasked first clear establishes every row byte even if
            // the destination is write-only. No read permission is synthesized.
            if row.buffer.len() != row_bytes
                || !matches!(
                    row.source,
                    CopyInput::Solid {
                        mask: 15,
                        previous: None,
                        ..
                    }
                )
            {
                return Err("Frame projection requires a full first clear".into());
            }
            let mut projected = row.buffer.clone();
            overlay(&row.target, &mut projected, &self.copies);
            if projected != expected {
                return Err("Raster oracle differs from prepared memory effects".into());
            }
        }
        Ok(())
    }
    /// All fallible validation/allocation finishes before any memory changes.
    /// Pins hold immutable permissions/lengths; the executor is single threaded,
    /// and no callbacks or borrowed storage escape. Exact-sized accesses below
    /// therefore cannot fail unless an internal pin invariant is broken.
    pub(crate) fn commit(mut self) -> (MethodState, ExecutionStats) {
        for copy in &mut self.copies {
            if !self.inputs_snapshotted {
                // Default execution-time reads see preceding published effects.
                // Explicit snapshots instead own all input-dependent bytes.
                copy.resolve(&[]).expect("prepared read pin");
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
            depth_inputs: Vec::new(),
            snapshot_bytes: 0,
            inputs_snapshotted: false,
            resolved_copies: 0,
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
                        depth: false,
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
        if batch.inputs_snapshotted {
            batch.snapshot_inputs()?;
        }
        if batch
            .depth_inputs
            .iter()
            .flat_map(|p| p.backing_ranges())
            .any(|a| {
                batch
                    .copies
                    .iter()
                    .filter(|c| !c.depth)
                    .flat_map(|c| c.target.backing_ranges())
                    .any(|b| overlaps(a, b))
            })
        {
            return Err(ExecutionError::DrawInputFeedback);
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
                for pixel in buffer.chunks_exact_mut(4) {
                    for component in 0..4 {
                        if mask & (1 << component) != 0 {
                            pixel[component] = color[component];
                        }
                    }
                }
                self.copies.push(Copy {
                    depth: false,
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
/// Resolve an ordered read without publishing any prepared writes. Backing IDs
/// and offsets, rather than virtual addresses, preserve aliases and split views.
/// Applying each target span in its write order also matches repeated backing
/// spans within a single target pin. No extra copy of the pending effects is made.
fn read_projected(
    source: &GpuMemoryRange,
    output: &mut [u8],
    copies: &[Copy],
) -> Result<(), ExecutionError> {
    source.read(output).map_err(ExecutionError::Memory)?;
    overlay(source, output, copies);
    Ok(())
}
fn overlay(source: &GpuMemoryRange, output: &mut [u8], copies: &[Copy]) {
    for copy in copies {
        let mut written = 0;
        for target in copy.target.backing_ranges() {
            let mut read = 0;
            for input in source.backing_ranges() {
                if overlaps(target, input) {
                    let start = target.offset.max(input.offset);
                    let end = (target.offset + target.length).min(input.offset + input.length);
                    let to = read + start - input.offset;
                    let from = written + start - target.offset;
                    output[to..to + end - start]
                        .copy_from_slice(&copy.buffer[from..from + end - start]);
                }
                read += input.length;
            }
            written += target.length;
        }
    }
}
impl PreparedBatch {
    fn add_draw(
        &mut self,
        space: &GpuAddressSpace,
        mut draw: crate::Draw,
        pins: Vec<GpuMemoryRange>,
        limits: ExecutionLimits,
        spans: &mut usize,
    ) -> Result<(), ExecutionError> {
        let mut depth_effect = None;
        let mut depth_capture = 0;
        if let Some(depth) = &mut draw.depth {
            self.snapshot_inputs()?;
            let source = space
                .pin(depth.address, 512, Access::Read)
                .map_err(ExecutionError::Memory)?;
            let mut raw = vec![0; 512];
            read_projected(&source, &mut raw, &self.copies)?;
            for y in 0..draw.surface.height as usize {
                for x in 0..draw.surface.width as usize {
                    let at = crate::depth::offset(x, y);
                    let value = u32::from_le_bytes(raw[at..at + 4].try_into().expect("depth word"));
                    if !f32::from_bits(value).is_finite()
                        || !(0.0..=1.0).contains(&f32::from_bits(value))
                    {
                        return Err(ExecutionError::UnsupportedDrawState);
                    }
                    depth.initial.push(value);
                }
            }
            *spans += source.backing_ranges().count();
            self.depth_inputs.push(source);
            depth_capture = 512 + depth.initial.len() * 4;
            if depth.write {
                let target = space
                    .pin(depth.address, 512, Access::Write)
                    .map_err(ExecutionError::Memory)?;
                *spans += target.backing_ranges().count();
                depth_effect = Some((target, raw));
            }
        }
        let (pixels, depth_values) = draw.raster_depth()?;
        let s = draw.surface;
        let bytes = pixels.iter().filter(|p| p.is_some()).count() * 4
            + if depth_effect.is_some() { 512 } else { 0 };
        let snapshots = pins.iter().map(GpuMemoryRange::len).sum::<usize>()
            + draw.texture.as_ref().map_or(0, |t| t.pixels.len())
            + draw.indices.as_ref().map_or(0, |i| i.values.len() * 4)
            + (draw.vertex_spirv.len() + draw.fragment_spirv.len()) * 4
            + depth_capture;
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
                    depth: false,
                    source: CopyInput::Prepared,
                    target,
                    buffer: (first..x)
                        .flat_map(|x| pixels[(y * s.width + x) as usize].expect("covered pixel"))
                        .collect(),
                });
            }
        }
        if let Some((target, mut raw)) = depth_effect {
            for y in 0..s.height as usize {
                for x in 0..s.width as usize {
                    let at = crate::depth::offset(x, y);
                    raw[at..at + 4]
                        .copy_from_slice(&depth_values[y * s.width as usize + x].to_le_bytes());
                }
            }
            self.copies.push(Copy {
                depth: true,
                source: CopyInput::Prepared,
                target,
                buffer: raw,
            });
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
