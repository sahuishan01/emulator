//! Owned indexed geometry: original NRO, scalar oracles and a separate value ABI.
use crate::{
    graphics::ClearOp,
    textures::{self, check, pixels, setup, wire, word, TextureCase, TexturedDraw},
};
use emulator_gpu::{ChannelProcessor, GpuVa};
use emulator_memory::{Access, Permissions};
use emulator_session::{RunOutcome, Session, LOAD_BASE};
pub(crate) const IMAGE: &[u8] = include_bytes!("../../../examples/homebrew/nv-indexed.nro");
pub(crate) const MESSAGE: &str = "Guest indices passed: shared vertices, rollback and lifetime.\n";
const PAGES: [&[u8]; 3] = [
    include_bytes!("../../../examples/gpu/indexed-u8-page.bin"),
    include_bytes!("../../../examples/gpu/indexed-u16-page.bin"),
    include_bytes!("../../../examples/gpu/indexed-u32-page.bin"),
];
const WORDS: u32 = include_bytes!("../../../examples/gpu/indexed-u8-commands.bin").len() as u32 / 4;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IndexedDraw {
    pub textured: TexturedDraw,
    pub index_count: u32,
    pub element_bytes: u32,
    pub base_vertex: i32,
    pub indices: [u32; 96],
}
impl Default for IndexedDraw {
    fn default() -> Self {
        Self {
            textured: TexturedDraw::default(),
            index_count: 0,
            element_bytes: 0,
            base_vertex: 0,
            indices: [0; 96],
        }
    }
}
#[repr(C)]
pub struct IndexedReplay {
    pub abi_version: u32,
    pub status: u32,
    pub draw_count: u32,
    pub image_bytes: u32,
    pub background: ClearOp,
    pub draws: [IndexedDraw; 3],
    pub pixels: [u8; 512],
    pub error: [u8; 256],
}
pub(crate) fn expected(index: usize) -> [u8; 512] {
    if index < 2 {
        return textures::expected(0);
    }
    // Integer pixel-center half-plane and texel formula, independent of raster IR.
    let mut out = textures::expected(2);
    let palette = [
        [255, 0, 0, 255],
        [255, 255, 0, 255],
        [0, 255, 0, 255],
        [0, 255, 255, 255],
    ];
    for y in 2..6i32 {
        for x in 4..12i32 {
            if x + 2 * y <= 14 {
                continue;
            }
            let sx = (6 * x - 13).div_euclid(16).clamp(0, 3);
            let sy = (6 * y - 5).div_euclid(8).clamp(0, 3);
            let a = ((y * 16 + x) * 4) as usize;
            out[a..a + 4].copy_from_slice(&palette[((sx + sy + 1) % 4) as usize]);
        }
    }
    out
}
pub(crate) fn verify_guest(s: &Session) -> Result<(), String> {
    let mut page = [0; 4096];
    s.memory()
        .read(LOAD_BASE + 0x5000, &mut page)
        .map_err(|e| e.to_string())?;
    check(
        page[0x800..0xa00] == expected(0)
            && page[..0x800] == PAGES[0][..0x800]
            && page[0xa00..] == PAGES[0][0xa00..],
    )?;
    check(
        s.memory()
            .read_u64(LOAD_BASE + 0x4040)
            .map_err(|e| e.to_string())?
            == 2
            && s.memory()
                .read_u64(LOAD_BASE + 0x3000)
                .map_err(|e| e.to_string())?
                == 42,
    )
}
fn guest() -> Result<Session, String> {
    let mut s = Session::from_nro(IMAGE).map_err(|e| e.to_string())?;
    check(
        s.run(10000).map_err(|e| e.to_string())? == RunOutcome::Exited
            && s.instructions() == 5571
            && s.debug_records() == [MESSAGE.as_bytes().to_vec()]
            && s.kernel().live_handle_count() == 0
            && s.kernel().live_transfer_memory_count() == 0
            && s.memory().lent_bytes() == 0,
    )?;
    verify_guest(&s)?;
    Ok(s)
}
pub fn replay(index: u32) -> IndexedReplay {
    let mut out = IndexedReplay {
        abi_version: 1,
        status: 1,
        draw_count: 0,
        image_bytes: 0,
        background: ClearOp::default(),
        draws: [IndexedDraw::default(); 3],
        pixels: [0; 512],
        error: [0; 256],
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(index, &mut out)))
        .unwrap_or_else(|_| Err("Indexed replay panic".into()));
    if let Err(e) = result {
        for (a, b) in out.error.iter_mut().zip(e.bytes().take(255)) {
            *a = b;
        }
    }
    out
}
fn fill(index: u32, out: &mut IndexedReplay) -> Result<(), String> {
    let mut page = PAGES
        .get(index as usize)
        .ok_or("Unknown indexed case")?
        .to_vec();
    if index == 0 {
        let s = guest()?;
        s.memory()
            .read(LOAD_BASE + 0x5000, &mut page)
            .map_err(|e| e.to_string())?;
    }
    let (mem, gpu) = setup(page)?;
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, WORDS))?;
    check(work.draws().len() == 2 && work.color_clears().len() == 1)?;
    out.background = work.color_clears()[0].into();
    for (out, source) in out.draws.iter_mut().zip(work.draws()) {
        let indices = source.indices.as_ref().ok_or("Missing indexed geometry")?;
        check(indices.values.len() <= 96)?;
        out.textured = textures::attributes(source)?;
        out.index_count = indices.values.len() as u32;
        out.element_bytes = indices.element_bytes;
        out.base_vertex = indices.base_vertex;
        out.indices[..indices.values.len()].copy_from_slice(&indices.values);
    }
    let stats = work.commit().1;
    check(
        stats.draw_launches == 2
            && stats.clear_launches == 1
            && stats.drawn_bytes == if index == 2 { 576 } else { 640 },
    )?;
    out.pixels = pixels(&mem)?;
    check(out.pixels == expected(index as usize))?;
    out.status = 0;
    out.draw_count = 2;
    out.image_bytes = 512;
    Ok(())
}
fn method(page: &mut [u8], offset: u32, value: u32) {
    let at = (0..WORDS as usize * 4)
        .step_by(8)
        .find(|i| u32::from_le_bytes(page[*i..*i + 4].try_into().unwrap()) & 0x1fff == offset / 4)
        .expect("owned fixture method");
    word(page, at + 4, value);
}
fn rejected(page: Vec<u8>, words: u32) -> Result<(), String> {
    let (mem, gpu) = setup(page)?;
    check(
        ChannelProcessor::default()
            .prepare(&gpu, &wire(0, words))
            .is_err(),
    )?;
    check(pixels(&mem)? == [0; 512])
}
fn core_case(i: usize) -> Result<usize, String> {
    match i {
        0 => {
            for n in 0..3 {
                let r = replay(n);
                if r.status != 0 {
                    return Err(String::from_utf8_lossy(&r.error).into());
                }
                check(r.pixels == expected(n as usize))?;
                let d = r.draws[0];
                check(
                    d.element_bytes == [1, 2, 4][n as usize]
                        && d.base_vertex == [0, -1, 1][n as usize]
                        && d.textured.draw.vertex_count == [4, 4, 5][n as usize],
                )?;
            }
            Ok(1536)
        }
        1 => {
            let mut count = 0;
            for (offset, value) in [
                (0x17c8, 256),
                (0x17cc, 0xf01),
                (0x17d4, 0xf0e),
                (0x17d0, 256),
                (0x1434, i32::MIN as u32),
                (0x1434, i32::MAX as u32),
                (0xde8, 1),
                (0xde8, 2),
                (0x17e8, 0x40060003),
                (0x17e8, 0x40000002),
                (0x17e8, 0x40630002),
                (0x17e8, 0x40040002),
                (0x17e8, 0x50060002),
                (0x1f04, 0xd7e),
            ] {
                let mut page = PAGES[1].to_vec();
                method(&mut page, offset, value);
                rejected(page, WORDS)?;
                count += 1;
            }
            for v in [0u16, 97, 65535] {
                let mut page = PAGES[1].to_vec();
                page[0xf04..0xf06].copy_from_slice(&v.to_le_bytes());
                rejected(page, WORDS)?;
                count += 1;
            }
            for o in [0x17e0u32, 0x17f0, 0x17f4, 0x17f8] {
                let mut page = PAGES[0].to_vec();
                let at = (0..WORDS as usize * 4)
                    .step_by(8)
                    .find(|a| {
                        u32::from_le_bytes(page[*a..*a + 4].try_into().unwrap()) & 0x1fff
                            == 0x17ec / 4
                    })
                    .unwrap();
                word(&mut page, at, 0x20010000 | (o / 4));
                rejected(page, WORDS)?;
                count += 1;
            }
            Ok(count)
        }
        2 => {
            rejected(PAGES[0].to_vec(), WORDS + 2)?;
            Ok(512)
        }
        3 => {
            for alias in [false, true] {
                let mut page = PAGES[0].to_vec();
                method(&mut page, 0x17cc, 0x800);
                method(&mut page, 0x17d4, 0x805);
                if alias {
                    method(&mut page, 0x17c8, 2);
                    method(&mut page, 0x17d0, 2);
                }
                let (mem, mut gpu) = setup(page)?;
                if alias {
                    gpu.map(
                        GpuVa(2 << 32),
                        mem.pin(0x1000, 4096, Permissions::READ_WRITE)
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                }
                check(
                    ChannelProcessor::default()
                        .prepare(&gpu, &wire(0, WORDS))
                        .unwrap_err()
                        .contains("DrawInputFeedback")
                        && pixels(&mem)? == [0; 512],
                )?;
            }
            Ok(1024)
        }
        4 => {
            let (mut mem, mut gpu) = setup(PAGES[0].to_vec())?;
            let original = mem
                .pin(0x1000, 4096, Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            let (_, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, WORDS))?;
            mem.write(0x1f00, &[255; 6]).map_err(|e| e.to_string())?;
            check(work.draws()[0].indices.as_ref().unwrap().values == [0, 1, 2, 2, 1, 3])?;
            let id = gpu
                .pin(GpuVa(1 << 32), 4, Access::Read)
                .map_err(|e| e.to_string())?
                .mapping_ids()
                .next()
                .ok_or("No mapping")?;
            gpu.unmap(id).map_err(|e| e.to_string())?;
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
            let mut old = [0; 512];
            original.read(0x800, &mut old).map_err(|e| e.to_string())?;
            check(old == expected(0) && pixels(&mem)? == [0xcc; 512])?;
            gpu.unmap(replacement).map_err(|e| e.to_string())?;
            mem.unmap(0x1000, 4096).map_err(|e| e.to_string())?;
            drop(original);
            check(mem.resident_bytes() == 0)?;
            Ok(512)
        }
        5 => {
            let (mut mem, gpu) = setup(PAGES[0].to_vec())?;
            let (next, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, WORDS))?;
            work.commit();
            mem.write(0x1f00, &[0; 6]).map_err(|e| e.to_string())?;
            let (_, work) = next.prepare(&gpu, &wire(0x400, 2))?;
            check(work.draws()[0].indices.as_ref().unwrap().values == [0; 6])?;
            let stats = work.commit().1;
            check(
                stats.drawn_bytes == 0 && stats.draw_launches == 1 && pixels(&mem)? == expected(0),
            )?;
            mem.write(0x1f00, &[0, 1, 2, 2, 1, 3])
                .map_err(|e| e.to_string())?;
            let (_, work) = next.prepare(&gpu, &wire(0x400, 2))?;
            check(work.commit().1.drawn_bytes == 128)?;
            Ok(512)
        }
        6 => {
            for start in [0xffcu32, 0x1000] {
                let mut page = PAGES[0].to_vec();
                method(&mut page, 0x17cc, start);
                method(&mut page, 0x17d4, start + 5);
                rejected(page, WORDS)?;
            }
            let mut page = PAGES[0].to_vec();
            method(&mut page, 0x17c8, 2);
            method(&mut page, 0x17d0, 2);
            let (mut mem, mut gpu) = setup(page)?;
            mem.map(0x3000, vec![0; 4096], Permissions::WRITE)
                .map_err(|e| e.to_string())?;
            gpu.map(
                GpuVa(2 << 32),
                mem.pin(0x3000, 4096, Permissions::WRITE)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &wire(0, WORDS))
                    .is_err()
                    && pixels(&mem)? == [0; 512],
            )?;
            Ok(3)
        }
        7 => {
            let mut bad = PAGES[0].to_vec();
            let at = (0..WORDS as usize * 4)
                .step_by(8)
                .rfind(|a| {
                    u32::from_le_bytes(bad[*a..*a + 4].try_into().unwrap()) & 0x1fff == 0x17ec / 4
                })
                .unwrap();
            word(&mut bad, at + 4, 0x400600ff);
            rejected(bad, WORDS)?;
            Ok(512)
        }
        8 => {
            guest()?;
            Ok(512)
        }
        _ => Err("Unknown indexed core case".into()),
    }
}
pub(crate) fn cases() -> Vec<TextureCase> {
    [
        "indexed-formats-first-and-signed-base",
        "indexed-invalid-ranges-modes-and-base",
        "indexed-late-command-rollback",
        "indexed-direct-and-alias-feedback",
        "indexed-capture-mutation-unmap-and-reuse",
        "indexed-retained-state-recaptures-elements",
        "indexed-unmapped-unreadable-and-incomplete-input",
        "indexed-second-draw-rollback",
        "indexed-guest-nro-rollback-and-lifetime",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        let r = std::panic::catch_unwind(|| core_case(i))
            .unwrap_or_else(|_| Err("Indexed core panic".into()));
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
    fn indexed_bridge_and_all_portable_cases() {
        use super::*;
        assert_eq!(std::mem::size_of::<IndexedDraw>(), 33424);
        assert_eq!(std::mem::offset_of!(IndexedDraw, indices), 33040);
        assert_eq!(std::mem::size_of::<IndexedReplay>(), 101116);
        assert_eq!(std::mem::align_of::<IndexedReplay>(), 4);
        assert_eq!(std::mem::offset_of!(IndexedReplay, draws), 76);
        assert_eq!(std::mem::offset_of!(IndexedReplay, pixels), 100348);
        assert_eq!(std::mem::offset_of!(IndexedReplay, error), 100860);
        for c in cases() {
            assert!(c.passed, "{}: {}", c.name, c.error);
        }
        let r = replay(3);
        assert_ne!(r.status, 0);
        assert_eq!((r.draw_count, r.image_bytes), (0, 0));
    }
}
