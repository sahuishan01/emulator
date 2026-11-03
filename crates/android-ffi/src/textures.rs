//! Original guest texture corpus, scalar oracles and bounded Vulkan value bridge.
use crate::{draws::DrawOp, graphics::ClearOp};
use emulator_gpu::{ChannelProcessor, GpuAddressSpace, GpuVa};
use emulator_memory::{AddressSpace, Permissions};
use emulator_session::{RunOutcome, Session, LOAD_BASE};
pub(crate) const IMAGE: &[u8] = include_bytes!("../../../examples/homebrew/nv-texture.nro");
pub(crate) const MESSAGE: &str =
    "Guest textures passed: coordinates, samples, rollback and lifetime.\n";
const PAGES: [&[u8]; 3] = [
    include_bytes!("../../../examples/gpu/texture-rgba-page.bin"),
    include_bytes!("../../../examples/gpu/texture-bgra-page.bin"),
    include_bytes!("../../../examples/gpu/texture-updated-page.bin"),
];
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TexturedDraw {
    pub draw: DrawOp,
    pub varyings: [u32; 24],
    pub texture_width: u32,
    pub texture_height: u32,
    pub texture_bytes: u32,
    pub texture: [u8; 16384],
}
impl Default for TexturedDraw {
    fn default() -> Self {
        Self {
            draw: DrawOp::default(),
            varyings: [0; 24],
            texture_width: 0,
            texture_height: 0,
            texture_bytes: 0,
            texture: [0; 16384],
        }
    }
}
#[repr(C)]
pub struct TextureReplay {
    pub abi_version: u32,
    pub status: u32,
    pub draw_count: u32,
    pub image_bytes: u32,
    pub background: ClearOp,
    pub draws: [TexturedDraw; 3],
    pub pixels: [u8; 512],
    pub error: [u8; 256],
}
pub(crate) fn expected(index: usize) -> [u8; 512] {
    let palette = [
        [255, 0, 0, 255],
        [255, 255, 0, 255],
        [0, 255, 0, 255],
        [0, 255, 255, 255],
    ];
    let mut out = [0; 512];
    for y in 0..8i32 {
        for x in 0..16i32 {
            let center = (4..12).contains(&x) && (2..6).contains(&y);
            let (sx, sy) = if center {
                ((6 * x - 29).div_euclid(8), (6 * y - 13).div_euclid(4))
            } else {
                ((6 * x - 13).div_euclid(16), (6 * y - 5).div_euclid(8))
            };
            let mut p =
                palette[(sx.clamp(0, 3) + sy.clamp(0, 3) + i32::from(index == 2)) as usize % 4];
            if index == 1 {
                p.swap(0, 2);
            }
            out[((y * 16 + x) * 4) as usize..((y * 16 + x + 1) * 4) as usize].copy_from_slice(&p);
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
        return Err("Guest texture pixels/fences/transfer marker mismatch".into());
    }
    let mut page = [0; 4096];
    s.memory()
        .read(LOAD_BASE + 0x5000, &mut page)
        .map_err(|e| e.to_string())?;
    if page[..0x800] != PAGES[0][..0x800] || page[0xa00..] != PAGES[0][0xa00..] {
        return Err("Guest texture inputs changed".into());
    }
    Ok(())
}
pub(crate) fn wire(offset: u32, words: u32) -> Vec<u8> {
    [offset, 1 | (words << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}
pub(crate) fn setup(page: Vec<u8>) -> Result<(AddressSpace, GpuAddressSpace), String> {
    let size = page.len();
    let mut mem = AddressSpace::default();
    mem.map(0x1000, page, Permissions::READ_WRITE)
        .map_err(|e| e.to_string())?;
    let mut gpu = GpuAddressSpace::new(size * 2).map_err(|e| e.to_string())?;
    gpu.map(
        GpuVa(1 << 32),
        mem.pin(0x1000, size, Permissions::READ_WRITE)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok((mem, gpu))
}
pub fn replay(index: u32) -> TextureReplay {
    let mut out = TextureReplay {
        abi_version: 1,
        status: 1,
        draw_count: 0,
        image_bytes: 0,
        background: ClearOp::default(),
        draws: [TexturedDraw::default(); 3],
        pixels: [0; 512],
        error: [0; 256],
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(index, &mut out)))
        .unwrap_or_else(|_| Err("Texture replay panic".into()));
    if let Err(e) = result {
        for (a, b) in out.error.iter_mut().zip(e.bytes().take(255)) {
            *a = b;
        }
    }
    out
}
fn fill(index: u32, out: &mut TextureReplay) -> Result<(), String> {
    if index >= 3 {
        return super::projection::fill_texture(index - 3, out);
    }
    let mut page = PAGES
        .get(index as usize)
        .ok_or("Unknown texture case")?
        .to_vec();
    if index == 0 {
        let mut s = Session::from_nro(IMAGE).map_err(|e| e.to_string())?;
        if s.run(100000).map_err(|e| e.to_string())? != RunOutcome::Exited
            || s.instructions() != 5571
            || s.debug_records() != [MESSAGE.as_bytes().to_vec()]
            || s.kernel().live_handle_count() != 0
            || s.kernel().live_transfer_memory_count() != 0
            || s.memory().lent_bytes() != 0
        {
            return Err("Texture guest execution/lifetime mismatch".into());
        }
        verify_guest(&s)?;
        s.memory()
            .read(LOAD_BASE + 0x5000, &mut page)
            .map_err(|e| e.to_string())?;
    }
    let (memory, gpu) = setup(page)?;
    let (_, work) = ChannelProcessor::default()
        .prepare(&gpu, &wire(0, 142))
        .map_err(|e| e.to_string())?;
    if work.draws().len() != 2 || work.color_clears().len() != 1 {
        return Err("Texture plan count mismatch".into());
    }
    out.background = work.color_clears()[0].into();
    for (out, source) in out.draws.iter_mut().zip(work.draws()) {
        if source.indices.is_some() {
            return Err("Texture ABI cannot carry indices".into());
        }
        *out = attributes(source)?;
    }
    let stats = work.commit().1;
    if stats.draw_launches != 2 || stats.clear_launches != 1 || stats.drawn_bytes != 640 {
        return Err("Texture effect stats mismatch".into());
    }
    memory
        .read(0x1800, &mut out.pixels)
        .map_err(|e| e.to_string())?;
    if out.pixels != expected(index as usize) {
        return Err("Texture independent pixel formula mismatch".into());
    }
    out.status = 0;
    out.draw_count = 2;
    out.image_bytes = 512;
    Ok(())
}
/// Captured shader/vertex/texture fields shared by the separate indexed ABI.
pub(crate) fn attributes(source: &emulator_gpu::Draw) -> Result<TexturedDraw, String> {
    let mut value = TexturedDraw::default();
    let out = &mut value;
    let t = source.texture.as_ref().ok_or("Missing sampled texture")?;
    if source.vertices.is_empty()
        || source.vertices.len() > 6
        || source.varying_inputs.len() != source.vertices.len()
        || t.pixels.len() > 16384
        || source.vertex_spirv.len() > 2048
        || source.fragment_spirv.len() > 2048
    {
        return Err("Texture ABI bounds".into());
    }
    let s = source.surface;
    let d = &mut out.draw;
    d.address_upper = (s.address.0 >> 32) as u32;
    d.address_lower = s.address.0 as u32;
    d.width = s.width;
    d.height = s.height;
    d.pitch = s.pitch;
    d.format = s.format;
    d.rect = s.rect;
    d.mask = s.mask;
    d.vertex_count = source.vertices.len() as u32;
    d.vertex_words = source.vertex_spirv.len() as u32;
    d.fragment_words = source.fragment_spirv.len() as u32;
    d.vertex[..source.vertex_spirv.len()].copy_from_slice(&source.vertex_spirv);
    d.fragment[..source.fragment_spirv.len()].copy_from_slice(&source.fragment_spirv);
    for (v, input) in d.vertices.chunks_exact_mut(4).zip(&source.vertices) {
        v.copy_from_slice(input);
    }
    for (v, input) in out.varyings.chunks_exact_mut(4).zip(&source.varying_inputs) {
        v.copy_from_slice(input);
    }
    out.texture_width = t.width;
    out.texture_height = t.height;
    out.texture_bytes = t.pixels.len() as u32;
    out.texture[..t.pixels.len()].copy_from_slice(&t.pixels);
    Ok(value)
}
pub(crate) struct TextureCase {
    pub name: &'static str,
    pub passed: bool,
    pub checks: usize,
    pub error: String,
}
pub(crate) fn check(ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err("Texture core oracle mismatch".into())
    }
}
pub(crate) fn word(page: &mut [u8], at: usize, value: u32) {
    page[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
pub(crate) fn pixels(memory: &AddressSpace) -> Result<[u8; 512], String> {
    let mut p = [0; 512];
    memory.read(0x1800, &mut p).map_err(|e| e.to_string())?;
    Ok(p)
}
fn core_case(i: usize) -> Result<usize, String> {
    match i {
        0 => {
            for n in 0..3 {
                let r = replay(n);
                check(r.status == 0 && r.pixels == expected(n as usize))?;
            }
            Ok(3 * 512)
        }
        1 => {
            let mut count = 0;
            for (at, value) in [
                (0x600, 0),
                (0x608, 3 << 21),
                (0x60c, 0),
                (0x610, (1 << 23) | 64),
                (0x614, 3),
                (0x618, 1),
                (0x61c, 1),
                (0x700, 0),
                (0x704, 0x52),
                (0x708, 1),
                (0x1c, 0),
            ] {
                let mut p = PAGES[0].to_vec();
                word(&mut p, at, value);
                let (mem, gpu) = setup(p)?;
                check(
                    ChannelProcessor::default()
                        .prepare(&gpu, &wire(0, 142))
                        .is_err(),
                )?;
                check(pixels(&mem)? == [0; 512])?;
                count += 1;
            }
            Ok(count)
        }
        2 => {
            let (mem, gpu) = setup(PAGES[0].to_vec())?;
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &wire(0, 144))
                    .is_err(),
            )?;
            check(pixels(&mem)? == [0; 512])?;
            Ok(512)
        }
        3 | 4 => {
            let mut page = PAGES[0].to_vec();
            word(&mut page, 0x604, 0x800);
            word(&mut page, 0x608, (2 << 21) | if i == 3 { 1 } else { 2 });
            let (mem, mut gpu) = setup(page)?;
            if i == 4 {
                gpu.map(
                    GpuVa(2 << 32),
                    mem.pin(0x1000, 4096, Permissions::READ_WRITE)
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            }
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &wire(0, 142))
                    .is_err(),
            )?;
            check(pixels(&mem)? == [0; 512])?;
            Ok(512)
        }
        5 => {
            let (mut mem, mut gpu) = setup(PAGES[0].to_vec())?;
            let original = mem
                .pin(0x1000, 4096, Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            let (_, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, 142))?;
            let original_map = gpu
                .pin(GpuVa(1 << 32), 4, emulator_memory::Access::Read)
                .map_err(|e| e.to_string())?
                .mapping_ids()
                .next()
                .ok_or("Missing mapping")?;
            gpu.unmap(original_map).map_err(|e| e.to_string())?;
            mem.unmap(0x1000, 4096).map_err(|e| e.to_string())?;
            mem.map(0x1000, vec![0xcc; 4096], Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            let replacement = gpu
                .map(
                    GpuVa(1 << 32),
                    mem.pin(0x1000, 4096, Permissions::READ_WRITE)
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            work.commit();
            let mut old = [0; 4096];
            original.read(0, &mut old).map_err(|e| e.to_string())?;
            check(old[0x800..0xa00] == expected(0))?;
            check(pixels(&mem)? == [0xcc; 512])?;
            gpu.unmap(replacement).map_err(|e| e.to_string())?;
            mem.unmap(0x1000, 4096).map_err(|e| e.to_string())?;
            drop(original);
            check(mem.resident_bytes() == 0)?;
            Ok(512)
        }
        6 => {
            let (mut mem, gpu) = setup(PAGES[0].to_vec())?;
            let (next, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, 142))?;
            work.commit();
            check(pixels(&mem)? == expected(0))?;
            mem.write(0x1500, &PAGES[2][0x500..0x580])
                .map_err(|e| e.to_string())?;
            let (_, work) = next.prepare(&gpu, &wire(0x400, 2))?;
            let stats = work.commit().1;
            check(
                stats.clear_launches == 0 && stats.draw_launches == 1 && stats.drawn_bytes == 128,
            )?;
            let mut oracle = expected(0);
            let updated = expected(2);
            for y in 2..6 {
                for x in 4..12 {
                    let a = (y * 16 + x) * 4;
                    oracle[a..a + 4].copy_from_slice(&updated[a..a + 4]);
                }
            }
            check(pixels(&mem)? == oracle)?;
            Ok(512)
        }
        7 => {
            let mut page = PAGES[0].to_vec();
            word(&mut page, 0x608, (2 << 21) | 0xff);
            let (mem, gpu) = setup(page)?;
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &wire(0, 142))
                    .is_err(),
            )?;
            check(pixels(&mem)? == [0; 512])?;
            Ok(1)
        }
        _ => Err("Unknown texture core case".into()),
    }
}
pub(crate) fn cases() -> Vec<TextureCase> {
    [
        "sampled-texture-three-independent-images",
        "sampled-descriptor-modes-and-bounds-rejected",
        "sampled-late-command-rollback",
        "sampled-target-feedback-rejected",
        "sampled-backing-alias-feedback-rejected",
        "sampled-input-capture-unmap-and-reuse",
        "sampled-retained-state-changed-source",
        "sampled-unmapped-texture-rejected",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        let r = std::panic::catch_unwind(|| core_case(i))
            .unwrap_or_else(|_| Err("Texture core panic".into()));
        TextureCase {
            name,
            passed: r.is_ok(),
            checks: r.as_ref().copied().unwrap_or(0),
            error: r.err().unwrap_or_default(),
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn guest_and_descriptor_texture_paths_match_independent_pixels() {
        assert_eq!(std::mem::size_of::<super::TexturedDraw>(), 33028);
        assert_eq!(std::mem::align_of::<super::TexturedDraw>(), 4);
        assert_eq!(std::mem::offset_of!(super::TexturedDraw, varyings), 16536);
        assert_eq!(std::mem::offset_of!(super::TexturedDraw, texture), 16644);
        assert_eq!(std::mem::size_of::<super::TextureReplay>(), 99928);
        assert_eq!(std::mem::align_of::<super::TextureReplay>(), 4);
        assert_eq!(std::mem::offset_of!(super::TextureReplay, draws), 76);
        assert_eq!(std::mem::offset_of!(super::TextureReplay, pixels), 99160);
        assert_eq!(std::mem::offset_of!(super::TextureReplay, error), 99672);
        for i in 0..3 {
            let r = super::replay(i);
            assert_eq!(r.status, 0, "{}", String::from_utf8_lossy(&r.error));
            assert_eq!(r.pixels, super::expected(i as usize));
        }
        let r = super::replay(11);
        assert_ne!(r.status, 0);
        assert_eq!(r.draw_count, 0);
        assert_eq!(r.image_bytes, 0);
    }
}
