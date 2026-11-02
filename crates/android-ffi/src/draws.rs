//! Captured owned guest commands -> original headers/shaders/draw plans -> replay.
use crate::graphics::ClearOp;
use emulator_gpu::{ChannelProcessor, GpuAddressSpace, GpuVa};
use emulator_memory::{AddressSpace, Permissions};
use emulator_session::{RunOutcome, Session, LOAD_BASE};
pub(crate) const IMAGE: &[u8] = include_bytes!("../../../examples/homebrew/nv-draw.nro");
pub(crate) const MESSAGE: &str =
    "Guest draws passed: headers, shaders, pixels, rollback and lifetime.\n";
const PAGES: [&[u8]; 3] = [
    include_bytes!("../../../examples/gpu/draw-rgba-page.bin"),
    include_bytes!("../../../examples/gpu/draw-bgra-page.bin"),
    include_bytes!("../../../examples/gpu/draw-scissor-page.bin"),
];
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrawOp {
    pub address_upper: u32,
    pub address_lower: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: u32,
    pub rect: [u32; 4],
    pub mask: u32,
    pub vertex_count: u32,
    pub vertex_words: u32,
    pub fragment_words: u32,
    pub vertex: [u32; 2048],
    pub fragment: [u32; 2048],
    pub vertices: [u32; 24],
}
impl Default for DrawOp {
    fn default() -> Self {
        Self {
            address_upper: 0,
            address_lower: 0,
            width: 0,
            height: 0,
            pitch: 0,
            format: 0,
            rect: [0; 4],
            mask: 0,
            vertex_count: 0,
            vertex_words: 0,
            fragment_words: 0,
            vertex: [0; 2048],
            fragment: [0; 2048],
            vertices: [0; 24],
        }
    }
}
#[repr(C)]
pub struct DrawReplay {
    pub abi_version: u32,
    pub status: u32,
    pub draw_count: u32,
    pub image_bytes: u32,
    pub background: ClearOp,
    pub draws: [DrawOp; 3],
    pub pixels: [u8; 512],
    pub error: [u8; 256],
}
pub(crate) fn expected(index: usize) -> [u8; 512] {
    let mut out = [0; 512];
    for y in 0..8 {
        for x in 0..16 {
            let mut p = if (4..12).contains(&x) && (2..6).contains(&y) && (index != 2 || x < 8) {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            };
            if index == 1 {
                p.swap(0, 2);
            }
            out[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].copy_from_slice(&p);
        }
    }
    out
}
pub(crate) fn verify_guest(s: &Session) -> Result<(), String> {
    let mut pixels = [0; 512];
    s.memory()
        .read(LOAD_BASE + 0x5800, &mut pixels)
        .map_err(|e| e.to_string())?;
    if pixels != expected(0)
        || s.memory()
            .read_u64(LOAD_BASE + 0x4040)
            .map_err(|e| e.to_string())?
            != 2
        || s.memory()
            .read_u64(LOAD_BASE + 0x3000)
            .map_err(|e| e.to_string())?
            != 42
    {
        return Err("Guest draw image/completion/transfer oracle mismatch".into());
    }
    let mut actual = [0; 4096];
    s.memory()
        .read(LOAD_BASE + 0x5000, &mut actual)
        .map_err(|e| e.to_string())?;
    if actual[..0x800] != PAGES[0][..0x800] || actual[0xa00..] != PAGES[0][0xa00..] {
        return Err("Guest draw commands/shader headers/code/vertices changed".into());
    }
    Ok(())
}
pub fn replay(index: u32) -> DrawReplay {
    let mut out = DrawReplay {
        abi_version: 1,
        status: 1,
        draw_count: 0,
        image_bytes: 0,
        background: ClearOp::default(),
        draws: [DrawOp::default(); 3],
        pixels: [0; 512],
        error: [0; 256],
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(index, &mut out)))
        .unwrap_or_else(|_| Err("Guest draw bridge panic".into()));
    if let Err(error) = result {
        out.draw_count = 0;
        out.image_bytes = 0;
        let len = error.len().min(255);
        out.error[..len].copy_from_slice(&error.as_bytes()[..len]);
    }
    out
}
fn build(index: u32, out: &mut DrawReplay) -> Result<(), String> {
    let Some(page) = PAGES.get(index as usize) else {
        return Err("Unknown guest draw fixture".into());
    };
    let mut page = page.to_vec();
    if index == 0 {
        let mut s = Session::from_nro(IMAGE).map_err(|e| e.to_string())?;
        if s.run(10000).map_err(|e| e.to_string())? != RunOutcome::Exited
            || s.kernel().live_handle_count() != 0
            || s.kernel().live_transfer_memory_count() != 0
            || s.memory().lent_bytes() != 0
            || s.debug_records() != [MESSAGE.as_bytes().to_vec()]
        {
            return Err("Guest draw execution/lifetime/debug mismatch".into());
        }
        verify_guest(&s)?;
        s.memory()
            .read(LOAD_BASE + 0x5000, &mut page)
            .map_err(|e| e.to_string())?;
    }
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, page, Permissions::READ_WRITE)
        .map_err(|e| e.to_string())?;
    let mut space = GpuAddressSpace::new(4096).map_err(|e| e.to_string())?;
    space
        .map(
            GpuVa(1 << 32),
            memory
                .pin(0x1000, 4096, Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let entries: Vec<_> = [0u32, 1 | (126 << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let (_, work) = ChannelProcessor::default().prepare(&space, &entries)?;
    if work.draws().len() != 2 || work.color_clears().len() != 1 {
        return Err("Guest draw plan count mismatch".into());
    }
    out.background = work.color_clears()[0].into();
    for (out, draw) in out.draws.iter_mut().zip(work.draws()) {
        let s = draw.surface;
        if draw.vertices.len() != 6
            || draw.texture.is_some()
            || draw.indices.is_some()
            || draw.vertex.varying_outputs().is_some()
            || draw.vertex_spirv.len() > 2048
            || draw.fragment_spirv.len() > 2048
        {
            return Err("Guest draw fixture ABI bounds".into());
        }
        out.address_upper = (s.address.0 >> 32) as u32;
        out.address_lower = s.address.0 as u32;
        out.width = s.width;
        out.height = s.height;
        out.pitch = s.pitch;
        out.format = s.format;
        out.rect = s.rect;
        out.mask = s.mask;
        out.vertex_count = 6;
        out.vertex_words = draw.vertex_spirv.len() as u32;
        out.fragment_words = draw.fragment_spirv.len() as u32;
        out.vertex[..draw.vertex_spirv.len()].copy_from_slice(&draw.vertex_spirv);
        out.fragment[..draw.fragment_spirv.len()].copy_from_slice(&draw.fragment_spirv);
        for (v, input) in out.vertices.chunks_exact_mut(4).zip(&draw.vertices) {
            v.copy_from_slice(input);
        }
    }
    let stats = work.commit().1;
    if stats.clear_launches != 1
        || stats.draw_launches != 2
        || stats.drawn_bytes != if index == 2 { 576 } else { 640 }
    {
        return Err("Guest draw effect statistics mismatch".into());
    }
    memory
        .read(0x1800, &mut out.pixels)
        .map_err(|e| e.to_string())?;
    if out.pixels != expected(index as usize) {
        return Err("Guest draw independent pixel oracle mismatch".into());
    }
    out.draw_count = 2;
    out.image_bytes = 512;
    out.status = 0;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn guest_draw_value_bridge_and_independent_pixels() {
        assert_eq!(std::mem::size_of::<super::DrawOp>(), 16536);
        assert_eq!(std::mem::size_of::<super::DrawReplay>(), 50452);
        assert_eq!(std::mem::align_of::<super::DrawReplay>(), 4);
        assert_eq!(std::mem::offset_of!(super::DrawReplay, draws), 76);
        assert_eq!(std::mem::offset_of!(super::DrawReplay, pixels), 49684);
        assert_eq!(std::mem::offset_of!(super::DrawReplay, error), 50196);
        for i in 0..3 {
            let r = super::replay(i);
            assert_eq!(r.status, 0, "{}", String::from_utf8_lossy(&r.error));
            assert_eq!(r.pixels, super::expected(i as usize));
        }
        let bad = super::replay(3);
        assert_ne!(bad.status, 0);
        assert_eq!((bad.draw_count, bad.image_bytes), (0, 0));
    }
}
