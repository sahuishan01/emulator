//! Owned command replay bridge. Vulkan consumes decoded clears, not guest shaders.
use emulator_gpu::{ChannelProcessor, ColorClear, GpuAddressSpace, GpuVa};
use emulator_memory::{AddressSpace, Permissions};
use emulator_session::{RunOutcome, Session, LOAD_BASE};

const FIXTURES: [&[u8]; 3] = [
    include_bytes!("../../../examples/gpu/color-clear-rgba.bin"),
    include_bytes!("../../../examples/gpu/color-clear-bgra.bin"),
    include_bytes!("../../../examples/gpu/color-clear-masked.bin"),
];
pub(crate) const IMAGE: &[u8] = include_bytes!("../../../examples/homebrew/nv-color-clear.nro");
pub(crate) const MESSAGE: &str =
    "Guest color clears passed: pixels, rollback, fences and lifetime.\n";

/// Independent rectangular image oracle; no method executor or float conversion.
fn expected(index: usize) -> [u8; 512] {
    let mut pixels = [0; 512];
    for y in 0..8 {
        for x in 0..16 {
            let mut pixel = if x < 8 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            if (4..12).contains(&x) && (2..6).contains(&y) {
                if index == 2 {
                    pixel[1] = 255;
                } else {
                    pixel = [0, 255, 0, 255];
                }
            }
            if index == 1 {
                pixel.swap(0, 2);
            }
            pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].copy_from_slice(&pixel);
        }
    }
    pixels
}
pub(crate) fn verify_guest(session: &Session) -> Result<(), String> {
    let memory = session.memory();
    let mut pixels = [0; 512];
    memory
        .read(LOAD_BASE + 0x5800, &mut pixels)
        .map_err(|e| e.to_string())?;
    if pixels != expected(0)
        || memory
            .read_u64(LOAD_BASE + 0x4040)
            .map_err(|e| e.to_string())?
            != 2
        || memory
            .read_u64(LOAD_BASE + 0x3000)
            .map_err(|e| e.to_string())?
            != 42
    {
        return Err("guest clear image/completion/transfer memory mismatch".into());
    }
    let mut commands = [0; 232];
    memory
        .read(LOAD_BASE + 0x5000, &mut commands)
        .map_err(|e| e.to_string())?;
    if commands != FIXTURES[0] {
        return Err("guest clear commands changed".into());
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ClearOp {
    pub address_upper: u32,
    pub address_lower: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: u32,
    pub rect: [u32; 4],
    pub color: [u32; 4],
    pub mask: u32,
}
impl From<ColorClear> for ClearOp {
    fn from(c: ColorClear) -> Self {
        Self {
            address_upper: (c.address.0 >> 32) as u32,
            address_lower: c.address.0 as u32,
            width: c.width,
            height: c.height,
            pitch: c.pitch,
            format: c.format,
            rect: c.rect,
            color: c.color,
            mask: c.mask,
        }
    }
}
#[repr(C)]
pub struct ClearReplay {
    pub abi_version: u32,
    pub status: u32,
    pub operation_count: u32,
    pub image_bytes: u32,
    pub operations: [ClearOp; 8],
    pub pixels: [u8; 512],
    pub error: [u8; 256],
}
impl Default for ClearReplay {
    fn default() -> Self {
        Self {
            abi_version: 1,
            status: 1,
            operation_count: 0,
            image_bytes: 0,
            operations: [ClearOp::default(); 8],
            pixels: [0; 512],
            error: [0; 256],
        }
    }
}
pub(crate) fn replay(index: u32) -> ClearReplay {
    let mut output = ClearReplay::default();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(index, &mut output)));
    let error = match result {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some("clear replay panic".into()),
    };
    if let Some(error) = error {
        output = ClearReplay::default();
        let n = error.len().min(output.error.len() - 1);
        output.error[..n].copy_from_slice(&error.as_bytes()[..n]);
    }
    output
}
fn build(index: u32, output: &mut ClearReplay) -> Result<(), String> {
    let index = usize::try_from(index).map_err(|e| e.to_string())?;
    let mut commands = FIXTURES
        .get(index)
        .ok_or("unknown clear replay fixture")?
        .to_vec();
    if index == 0 {
        let mut session = Session::from_nro(IMAGE).map_err(|e| e.to_string())?;
        if session.run(10000).map_err(|e| e.to_string())? != RunOutcome::Exited
            || session.debug_records() != [MESSAGE.as_bytes().to_vec()]
            || session.kernel().live_handle_count() != 0
            || session.kernel().live_transfer_memory_count() != 0
            || session.memory().lent_bytes() != 0
        {
            return Err("clear replay guest failed execution or cleanup".into());
        }
        verify_guest(&session)?;
        // Replay the actual post-execution command bytes emitted by this NRO.
        session
            .memory()
            .read(LOAD_BASE + 0x5000, &mut commands)
            .map_err(|e| e.to_string())?;
    }
    let mut memory = AddressSpace::with_limit(4096);
    memory
        .map(0x1000, vec![0; 4096], Permissions::READ_WRITE)
        .map_err(|e| e.to_string())?;
    memory.write(0x1000, &commands).map_err(|e| e.to_string())?;
    let mut gpu = GpuAddressSpace::new(4096).map_err(|e| e.to_string())?;
    gpu.map(
        GpuVa(1 << 32),
        memory
            .pin(0x1000, 4096, Permissions::READ_WRITE)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let entry = [0_u32, 1 | ((commands.len() as u32 / 4) << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry)?;
    if work.color_clears().len() != 3 {
        return Err("clear replay operation count mismatch".into());
    }
    for (out, clear) in output.operations.iter_mut().zip(work.color_clears()) {
        *out = (*clear).into();
    }
    let (_, stats) = work.commit();
    if stats.clear_launches != 3 || stats.cleared_bytes != 896 {
        return Err("clear replay effect count mismatch".into());
    }
    memory
        .read(0x1800, &mut output.pixels)
        .map_err(|e| e.to_string())?;
    if output.pixels != expected(index) {
        return Err("clear replay reference pixels mismatch".into());
    }
    output.operation_count = 3;
    output.image_bytes = 512;
    output.status = 0;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_replay_matches_all_pixels_and_rejects_bad_fixture() {
        for i in 0..3 {
            let r = super::replay(i);
            assert_eq!(r.status, 0, "{}", String::from_utf8_lossy(&r.error));
            assert_eq!(r.pixels, super::expected(i as usize));
        }
        let bad = super::replay(3);
        assert_ne!(bad.status, 0);
        assert_eq!(bad.operation_count, 0);
        assert_eq!(std::mem::size_of::<super::ClearReplay>(), 1264);
        assert_eq!(std::mem::align_of::<super::ClearReplay>(), 4);
    }
}
