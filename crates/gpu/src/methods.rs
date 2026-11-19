//! Original synchronous DMA reference executor. Register addresses/fields follow
//! NVIDIA's public MAXWELL_DMA_COPY_A interface; no vendor implementation is used.
use crate::{GpuAddressSpace, GpuMemoryError, GpuMemoryRange, GpuVa, MethodWrite};
use emulator_memory::{Access, BackingRange};

pub const MAXWELL_DMA_COPY_A: u32 = 0xb0b5;
pub const MAXWELL_COMPUTE_B: u32 = 0xb1c0;
/// Graphics-engine classes without an executor yet: binding is accepted,
/// any class method written to them fails explicitly.
pub const KEPLER_INLINE_TO_MEMORY_B: u32 = 0xa140;
pub const FERMI_TWOD_A: u32 = 0x902d;
const GRAPHICS_CLASSES: [u32; 4] = [
    crate::MAXWELL_B,
    MAXWELL_COMPUTE_B,
    KEPLER_INLINE_TO_MEMORY_B,
    FERMI_TWOD_A,
];

/// Whether a channel whose allocated object context is `context` may bind
/// `class` with SET_OBJECT. A graphics context owns the whole graphics engine
/// (3D, compute, inline-to-memory, 2D) and may also bind the copy engine
/// (Z-A's NVN binds 0xb0b5 on subchannel 4 of its 3D channel); a copy-only
/// context binds only the copy class.
pub fn class_allowed(context: u32, class: u32) -> bool {
    if class == context || (class == MAXWELL_DMA_COPY_A && GRAPHICS_CLASSES.contains(&context)) {
        return true;
    }
    GRAPHICS_CLASSES.contains(&context) && GRAPHICS_CLASSES.contains(&class)
}

/// Configuration ranges of classes without an executor: 2D surface, clip,
/// ROP and pattern setup (below the solid-primitive methods from 0x580), CPU
/// pixel and memory blit setup (their triggers 0x838 / 0x8fc excluded) and
/// inline-to-memory geometry
/// (below LAUNCH_DMA 0x1b0). Writes here change nothing observable on their own.
fn configuration_only(class: u32, offset: u32) -> bool {
    match class {
        FERMI_TWOD_A => {
            offset == 0x100
                || (0x200..0x580).contains(&offset)
                // PIXELS_FROM_CPU setup (data trigger 0x838) and
                // PIXELS_FROM_MEMORY setup (blit trigger SRC_Y0_INT 0x8fc).
                || (0x800..0x838).contains(&offset)
                || (0x860..0x8fc).contains(&offset)
        }
        _ => false,
    }
}

const HOST_SEMAPHORE_A: u32 = 0x10;
const HOST_SEMAPHORE_D: u32 = 0x1c;
/// SEMAPHORED operation field (bits 4:0).
const SEM_ACQUIRE: u32 = 1;
const SEM_RELEASE: u32 = 2;
const SEM_ACQ_GEQ: u32 = 4;
const SEM_ACQ_AND: u32 = 8;
/// SEMAPHORED RELEASE_WFI (bit 20) and RELEASE_SIZE 4 bytes (bit 24); bit 12
/// ACQUIRE_SWITCH only affects scheduling while a wait is pending.
const SEM_KNOWN_BITS: u32 = 0x1f | 1 << 12 | 1 << 20 | 1 << 24;

/// Host (PFIFO) methods, byte offsets below 0x100, valid on every subchannel.
const HOST_NOP: u32 = 0x08;
const HOST_FB_FLUSH: u32 = 0x24;
const HOST_MEM_OP_A: u32 = 0x28;
const HOST_MEM_OP_B: u32 = 0x2c;
const HOST_WFI: u32 = 0x78;
/// MEM_OP_B operations (bits 31:27) that only maintain L2: L2_SYSMEM_INVALIDATE
/// and L2_FLUSH_DIRTY. The reference model has no GPU cache, so they are exact.
const MEM_OP_L2_SYSMEM_INVALIDATE: u32 = 0x0e;
const MEM_OP_L2_FLUSH_DIRTY: u32 = 0x10;

/// Execute a host method. Synchronous in-order execution makes NOP, WFI and
/// flushes exact; semaphores, syncpoints and other host methods are not
/// modelled yet and fail explicitly.
fn host_method(subchannel: u8, offset: u32, data: u32) -> Result<(), ExecutionError> {
    match offset {
        HOST_NOP | HOST_FB_FLUSH | HOST_WFI | HOST_MEM_OP_A => Ok(()),
        HOST_MEM_OP_B
            if matches!(
                data >> 27,
                MEM_OP_L2_SYSMEM_INVALIDATE | MEM_OP_L2_FLUSH_DIRTY
            ) =>
        {
            Ok(())
        }
        _ => Err(ExecutionError::UnsupportedMethod { subchannel, offset }),
    }
}
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
    /// LOAD_CONSTANT_BUFFER bytes (also counted in copied_bytes).
    pub constant_bytes: usize,
    /// Inline-to-memory and semaphore payload bytes (also in copied_bytes).
    pub inline_bytes: usize,
    pub semaphore_releases: usize,
    pub semaphore_acquires: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionError {
    InvalidLimits,
    WrongAddressSpace,
    NotFront,
    UnboundSubchannel(u8),
    UnsupportedClass(u32),
    UnsupportedMethod {
        subchannel: u8,
        offset: u32,
    },
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
    Macro(crate::macros::MacroError),
    /// A draw or clear after a 3D state write the model does not interpret
    /// (first such byte offset); rendering it would silently ignore state.
    UnmodelledState(u32),
    ConstantBuffer(&'static str),
    InlineToMemory(&'static str),
    InlineToMemoryLaunch(u32),
    Semaphore(&'static str),
    SemaphoreOperation(u32),
    /// Acquire that would wait forever in synchronous execution.
    SemaphoreAcquire {
        address: u64,
        value: u32,
        payload: u32,
    },
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
/// 3D-only state outside the draw/clear models.
#[derive(Clone, Debug, Default)]
struct ThreedExtra {
    threed: bool,
    /// CB_SIZE, CB_ADDRESS_HIGH, CB_ADDRESS_LOW, CB_POS (0x2380..=0x238c).
    constant: [Option<u32>; 4],
    /// First recorded 3D state write that the draw/clear models ignore.
    unmodelled: Option<u32>,
    i2m: crate::i2m::I2m,
}

const CB_SIZE: u32 = 0x2380;
const CB_POS: u32 = 0x238c;
const CB_DATA_FIRST: u32 = 0x2390;
const CB_DATA_LAST: u32 = 0x23cc;
/// 3D methods with effects beyond register state that are not modelled:
/// the report semaphore trigger (SET_REPORT_SEMAPHORE_D). They fail instead of being recorded.
/// Draw begin/end (VERTEX_BEGIN/END_GL 0x1614/0x1618) and report-counter reset
/// (0x1530) are included so a draw outside the modelled triggers cannot be
/// recorded as plain state.
const THREED_ACTIONS: [u32; 4] = [0x1b0c, 0x1530, 0x1614, 0x1618];
/// Compute methods with effects: LAUNCH (0x2bc) and the report semaphore
/// trigger (0x1b0c).
const COMPUTE_ACTIONS: [u32; 2] = [0x2bc, 0x1b0c];
/// Methods the draw model turns into draws.
const DRAW_TRIGGERS: [u32; 4] = [0x1214, 0x17e4, 0x17e8, 0x17ec];
/// Bytes one constant-buffer upload run may hold (CB_SIZE is at most 64 KiB).
const CB_MAX: u32 = 0x1_0000;

/// Next LOAD_CONSTANT_BUFFER word address; advances CB_POS by 4.
fn constant_upload(regs: &mut [Option<u32>; 4]) -> Result<u64, ExecutionError> {
    let [Some(size), Some(high), Some(low), Some(pos)] = *regs else {
        return Err(ExecutionError::ConstantBuffer(
            "upload before CB_SIZE/ADDRESS/POS",
        ));
    };
    if size > CB_MAX || pos % 4 != 0 || pos.checked_add(4).is_none_or(|end| end > size) {
        return Err(ExecutionError::ConstantBuffer(
            "upload outside the constant buffer",
        ));
    }
    if high > 0xff || low % 4 != 0 {
        return Err(ExecutionError::InvalidAddressUpper(high));
    }
    regs[3] = Some(pos + 4);
    Ok((u64::from(high) << 32 | u64::from(low)) + u64::from(pos))
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MethodState {
    channels: [Option<EngineState>; 8],
    macros: crate::macros::MacroState,
    /// SEMAPHOREA (address upper), SEMAPHOREB (lower), SEMAPHOREC (payload).
    semaphore: [Option<u32>; 3],
}
#[derive(Clone, Debug)]
enum EngineState {
    Dma(DmaRegisters),
    Color(
        crate::clear::ColorRegisters,
        Box<crate::draw::Registers>,
        Box<ThreedExtra>,
    ),
    /// Class without an executor: configuration writes are accepted (they
    /// have no effect until a trigger), every trigger fails explicitly.
    Unimplemented(u32),
    InlineToMemory(Box<crate::i2m::I2m>),
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
    depth_clears: Vec<crate::DepthClear>,
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
    pub(crate) fn depth_clears(&self) -> &[crate::DepthClear] {
        &self.depth_clears
    }
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
            depth_clears: Vec::new(),
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
        let expanded = batch.state.macros.expand(writes)?;
        let writes = &expanded[..];
        let mut spans = 0;
        // Contiguous LOAD_CONSTANT_BUFFER words not yet turned into a write.
        let mut cb_run: Option<(u64, Vec<u8>)> = None;
        for write in writes {
            let cb_data = (CB_DATA_FIRST..=CB_DATA_LAST).contains(&write.method_byte_offset());
            if !cb_data {
                if let Some(run) = cb_run.take() {
                    batch.flush_constant(space, run, &mut spans)?;
                }
            }
            let channel = &mut batch.state.channels[usize::from(write.subchannel)];
            let offset = write.method_byte_offset();
            if offset == 0 {
                let engine = match write.data {
                    MAXWELL_DMA_COPY_A => EngineState::Dma(DmaRegisters::default()),
                    crate::MAXWELL_B | MAXWELL_COMPUTE_B => EngineState::Color(
                        crate::clear::ColorRegisters::default(),
                        Box::default(),
                        Box::new(ThreedExtra {
                            threed: write.data == crate::MAXWELL_B,
                            ..ThreedExtra::default()
                        }),
                    ),
                    KEPLER_INLINE_TO_MEMORY_B => EngineState::InlineToMemory(Box::default()),
                    FERMI_TWOD_A => EngineState::Unimplemented(write.data),
                    _ => return Err(ExecutionError::UnsupportedClass(write.data)),
                };
                // This subset binds a class directly. NV object handles and
                // engine selection bits require the future NV object registry.
                *channel = Some(engine);
                continue;
            }
            if offset < 0x100 {
                if (HOST_SEMAPHORE_A..=HOST_SEMAPHORE_D).contains(&offset) {
                    batch.semaphore(space, offset, write.data, &mut spans)?;
                } else {
                    host_method(write.subchannel, offset, write.data)?;
                }
                continue;
            }
            let engine = channel
                .as_mut()
                .ok_or(ExecutionError::UnboundSubchannel(write.subchannel))?;
            if let EngineState::InlineToMemory(i2m) = engine {
                if crate::i2m::I2m::handles(offset) {
                    for (address, bytes) in i2m.write(offset, write.data)? {
                        batch.push_prepared(space, address, bytes, &mut spans, false)?;
                    }
                    continue;
                }
                i2m.require_idle()?;
                if offset == 0x100 {
                    continue;
                }
                return Err(ExecutionError::UnsupportedMethod {
                    subchannel: write.subchannel,
                    offset,
                });
            }
            if let EngineState::Unimplemented(class) = engine {
                if configuration_only(*class, offset) {
                    continue;
                }
                return Err(ExecutionError::UnsupportedMethod {
                    subchannel: write.subchannel,
                    offset,
                });
            }
            if let EngineState::Color(registers, draw_registers, extra) = engine {
                if crate::i2m::I2m::handles(offset) {
                    for (address, bytes) in extra.i2m.write(offset, write.data)? {
                        batch.push_prepared(space, address, bytes, &mut spans, false)?;
                    }
                    continue;
                }
                extra.i2m.require_idle()?;
                if !extra.threed {
                    // Compute: state only matters at LAUNCH, which is not
                    // modelled; the launch and report trigger fail explicitly.
                    if COMPUTE_ACTIONS.contains(&offset) {
                        return Err(ExecutionError::UnsupportedMethod {
                            subchannel: write.subchannel,
                            offset,
                        });
                    }
                    continue;
                }
                if extra.threed && (CB_SIZE..=CB_DATA_LAST).contains(&offset) {
                    if offset <= CB_POS {
                        extra.constant[((offset - CB_SIZE) / 4) as usize] = Some(write.data);
                    } else {
                        let address = constant_upload(&mut extra.constant)?;
                        cb_run = batch.append_constant(
                            space,
                            cb_run.take(),
                            address,
                            write.data,
                            &mut spans,
                        )?;
                    }
                    continue;
                }
                if let Some(first) = extra.unmodelled {
                    let launches = offset == 0x19d0 || DRAW_TRIGGERS.contains(&offset);
                    if launches {
                        return Err(ExecutionError::UnmodelledState(first));
                    }
                }
                if offset == 0x19d0 && write.data == 1 {
                    let clear = draw_registers.depth_clear(registers)?;
                    batch.add_depth_clear(space, clear, limits, &mut spans)?;
                    continue;
                }
                if let Some(command) = draw_registers.write(offset, write.data)? {
                    if let Some(command) = command {
                        let (draw, pins) = draw_registers.plan(space, registers, command)?;
                        batch.add_draw(space, draw, pins, limits, &mut spans)?;
                    }
                    continue;
                }
                let clear = match registers.write(offset, write.data) {
                    Ok(clear) => clear,
                    // Plain 3D state the models do not interpret is recorded
                    // (the macro register file keeps its value); later draws
                    // and clears then fail with UnmodelledState.
                    Err(ExecutionError::UnsupportedMethod { offset, .. })
                        if extra.threed && !THREED_ACTIONS.contains(&offset) =>
                    {
                        extra.unmodelled.get_or_insert(offset);
                        None
                    }
                    Err(ExecutionError::UnsupportedMethod { offset, .. }) => {
                        return Err(ExecutionError::UnsupportedMethod {
                            subchannel: write.subchannel,
                            offset,
                        })
                    }
                    Err(error) => return Err(error),
                };
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
            .chain(
                batch
                    .copies
                    .iter()
                    .filter(|c| c.depth)
                    .flat_map(|c| c.target.backing_ranges()),
            )
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
        if let Some(run) = cb_run.take() {
            batch.flush_constant(space, run, &mut spans)?;
        }
        for engine in batch.state.channels.iter().flatten() {
            match engine {
                EngineState::Color(_, _, extra) => extra.i2m.require_idle()?,
                EngineState::InlineToMemory(i2m) => i2m.require_idle()?,
                _ => {}
            }
        }
        // A draw or clear anywhere in a batch whose 3D state includes ignored
        // writes (earlier or later in the batch) is rejected as a whole.
        if !(batch.draws.is_empty() && batch.clears.is_empty() && batch.depth_clears.is_empty()) {
            if let Some(first) = batch.state.channels.iter().flatten().find_map(|e| match e {
                EngineState::Color(_, _, extra) => extra.unmodelled,
                _ => None,
            }) {
                return Err(ExecutionError::UnmodelledState(first));
            }
        }
        Ok(batch)
    }
}

impl PreparedBatch {
    /// Extend the pending constant-buffer run with one word, or flush it and
    /// start a new run when `address` is not contiguous.
    fn append_constant(
        &mut self,
        space: &GpuAddressSpace,
        run: Option<(u64, Vec<u8>)>,
        address: u64,
        data: u32,
        spans: &mut usize,
    ) -> Result<Option<(u64, Vec<u8>)>, ExecutionError> {
        let mut run = match run {
            Some((start, bytes)) if start + bytes.len() as u64 == address => (start, bytes),
            Some(other) => {
                self.flush_constant(space, other, spans)?;
                (address, Vec::new())
            }
            None => (address, Vec::new()),
        };
        run.1.extend_from_slice(&data.to_le_bytes());
        Ok(Some(run))
    }
    /// Host semaphore methods. Release writes the 4-byte payload in order with
    /// other prepared writes. Acquire reads the semaphore (including earlier
    /// writes of this batch); in synchronous execution nothing else can change
    /// it while the channel would wait, so an unsatisfied acquire fails.
    /// 16-byte releases (payload + GPU timestamp) and reductions fail.
    fn semaphore(
        &mut self,
        space: &GpuAddressSpace,
        offset: u32,
        data: u32,
        spans: &mut usize,
    ) -> Result<(), ExecutionError> {
        if offset != HOST_SEMAPHORE_D {
            self.state.semaphore[((offset - HOST_SEMAPHORE_A) / 4) as usize] = Some(data);
            return Ok(());
        }
        let [Some(high), Some(low), Some(payload)] = self.state.semaphore else {
            return Err(ExecutionError::Semaphore(
                "operation before address/payload",
            ));
        };
        if high > 0xff || low % 4 != 0 {
            return Err(ExecutionError::Semaphore("invalid address"));
        }
        if data & !SEM_KNOWN_BITS != 0 {
            return Err(ExecutionError::SemaphoreOperation(data));
        }
        let address = u64::from(high) << 32 | u64::from(low);
        match data & 0x1f {
            SEM_RELEASE => {
                if data & 1 << 24 == 0 {
                    return Err(ExecutionError::Semaphore(
                        "16-byte release needs a GPU timestamp",
                    ));
                }
                self.push_prepared(space, address, payload.to_le_bytes().to_vec(), spans, false)?;
                self.stats.semaphore_releases += 1;
            }
            op @ (SEM_ACQUIRE | SEM_ACQ_GEQ | SEM_ACQ_AND) => {
                let source = space
                    .pin(GpuVa(address), 4, Access::Read)
                    .map_err(ExecutionError::Memory)?;
                let mut value = [0; 4];
                read_projected(&source, &mut value, &self.copies)?;
                let value = u32::from_le_bytes(value);
                let satisfied = match op {
                    SEM_ACQUIRE => value == payload,
                    // GEQ compares as a wrapping sequence number.
                    SEM_ACQ_GEQ => (value.wrapping_sub(payload) as i32) >= 0,
                    _ => value & payload != 0,
                };
                if !satisfied {
                    return Err(ExecutionError::SemaphoreAcquire {
                        address,
                        value,
                        payload,
                    });
                }
                self.stats.semaphore_acquires += 1;
            }
            _ => return Err(ExecutionError::SemaphoreOperation(data)),
        }
        Ok(())
    }
    /// Turn a constant-buffer run into an ordered prepared write.
    fn flush_constant(
        &mut self,
        space: &GpuAddressSpace,
        (start, bytes): (u64, Vec<u8>),
        spans: &mut usize,
    ) -> Result<(), ExecutionError> {
        self.push_prepared(space, start, bytes, spans, true)
    }
    /// Queue an ordered write of known bytes (constant buffer or I2M data).
    fn push_prepared(
        &mut self,
        space: &GpuAddressSpace,
        start: u64,
        bytes: Vec<u8>,
        spans: &mut usize,
        constant: bool,
    ) -> Result<(), ExecutionError> {
        let total = self.stats.copied_bytes
            + self.stats.cleared_bytes
            + self.stats.drawn_bytes
            + self.snapshot_bytes;
        if bytes.len() > MAX_BYTES.saturating_sub(total) {
            return Err(ExecutionError::Capacity);
        }
        let target = space
            .pin(GpuVa(start), bytes.len(), Access::Write)
            .map_err(ExecutionError::Memory)?;
        *spans += target.backing_ranges().count();
        if *spans > crate::address_space::MAX_BACKING_SPANS {
            return Err(ExecutionError::Capacity);
        }
        self.copies
            .try_reserve(1)
            .map_err(|_| ExecutionError::Capacity)?;
        self.stats.copied_bytes += bytes.len();
        if constant {
            self.stats.constant_bytes += bytes.len();
        } else {
            self.stats.inline_bytes += bytes.len();
        }
        self.copies.push(Copy {
            depth: false,
            source: CopyInput::Prepared,
            target,
            buffer: bytes,
        });
        Ok(())
    }
    fn add_depth_clear(
        &mut self,
        space: &GpuAddressSpace,
        mut clear: crate::DepthClear,
        limits: ExecutionLimits,
        spans: &mut usize,
    ) -> Result<(), ExecutionError> {
        let empty = clear.rect[0] == clear.rect[2] || clear.rect[1] == clear.rect[3];
        let full = clear.width == 16 && clear.height == 8 && clear.rect == [0, 0, 16, 8];
        let bytes = if empty { 0 } else { 512 };
        let snapshot = if empty || full { 0 } else { 512 };
        if self.stats.copy_launches + self.stats.clear_launches + self.stats.draw_launches
            >= limits.launches
            || bytes + snapshot
                > limits.bytes.saturating_sub(
                    self.stats.copied_bytes
                        + self.stats.cleared_bytes
                        + self.stats.drawn_bytes
                        + self.snapshot_bytes,
                )
        {
            return Err(ExecutionError::Capacity);
        }
        self.snapshot_inputs()?;
        if !empty {
            let target = space
                .pin(clear.address, 512, Access::Write)
                .map_err(ExecutionError::Memory)?;
            let source = if full {
                None
            } else {
                Some(
                    space
                        .pin(clear.address, 512, Access::Read)
                        .map_err(ExecutionError::Memory)?,
                )
            };
            *spans += target.backing_ranges().count()
                + source.as_ref().map_or(0, |s| s.backing_ranges().count());
            if *spans > crate::address_space::MAX_BACKING_SPANS {
                return Err(ExecutionError::Capacity);
            }
            let mut raw = vec![0; 512];
            if let Some(source) = &source {
                read_projected(source, &mut raw, &self.copies)?;
            }
            for y in clear.rect[1]..clear.rect[3] {
                for x in clear.rect[0]..clear.rect[2] {
                    let at = crate::depth::offset(x as usize, y as usize);
                    raw[at..at + 4].copy_from_slice(&clear.value.to_le_bytes());
                }
            }
            if let Some(source) = source {
                self.depth_inputs.push(source);
            }
            self.copies.push(Copy {
                depth: true,
                source: CopyInput::Prepared,
                target,
                buffer: raw,
            });
        }
        clear.before_draw = self.draws.len() as u32;
        self.depth_clears.push(clear);
        self.stats.clear_launches += 1;
        self.stats.cleared_bytes += bytes;
        self.snapshot_bytes += snapshot;
        Ok(())
    }
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
