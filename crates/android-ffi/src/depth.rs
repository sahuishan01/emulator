//! Original depth command corpus, independent oracles and owned value ABI.
use crate::{
    draws::DrawOp,
    graphics::ClearOp,
    textures::{self, TextureCase},
};
use emulator_gpu::{ChannelProcessor, ExecutionLimits, GpuVa, QueueLimits, SubmissionQueue};
use emulator_memory::{Access, Permissions};
const WORDS: u32 = 162;
const PAGES: [&[u8]; 11] = [
    include_bytes!("../../../examples/gpu/depth-0-page.bin"),
    include_bytes!("../../../examples/gpu/depth-1-page.bin"),
    include_bytes!("../../../examples/gpu/depth-2-page.bin"),
    include_bytes!("../../../examples/gpu/depth-3-page.bin"),
    include_bytes!("../../../examples/gpu/depth-4-page.bin"),
    include_bytes!("../../../examples/gpu/depth-5-page.bin"),
    include_bytes!("../../../examples/gpu/depth-6-page.bin"),
    include_bytes!("../../../examples/gpu/depth-7-page.bin"),
    include_bytes!("../../../examples/gpu/depth-8-page.bin"),
    include_bytes!("../../../examples/gpu/depth-9-page.bin"),
    include_bytes!("../../../examples/gpu/depth-10-page.bin"),
];
#[repr(C)]
pub struct DepthReplay {
    pub abi_version: u32,
    pub status: u32,
    pub draw_count: u32,
    pub image_bytes: u32,
    pub background: ClearOp,
    pub draws: [DrawOp; 3],
    pub pixels: [u8; 512],
    pub error: [u8; 256],
    pub initial: [u32; 128],
    pub expected: [u32; 128],
    pub functions: [u32; 3],
    pub writes: [u32; 3],
}
fn check(ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err("Depth oracle mismatch".into())
    }
}
fn expected(i: usize) -> ([u8; 512], [u32; 128]) {
    let mut color = [0; 512];
    let mut depth = [0; 128];
    for y in 0..8 {
        for x in 0..16 {
            let center = (4..12).contains(&x) && (2..6).contains(&y);
            let red = center && ([3, 6, 7, 8].contains(&i) || i >= 9 && x >= 8);
            let pixel = if i == 0 {
                [0, 0, 0, 255]
            } else if red {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            };
            color[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].copy_from_slice(&pixel);
            let z: f32 = match i {
                0 | 8 => 1.,
                1 | 5 => 0.25,
                2 | 3 | 6 => 0.5,
                4 => 0.75,
                7 => {
                    if center {
                        0.75
                    } else {
                        0.25
                    }
                }
                9 | 10 => {
                    if red {
                        0.5
                    } else {
                        let z = (2 * x + 1) as f32 / 32.;
                        if i == 9 {
                            z
                        } else {
                            1. - z
                        }
                    }
                }
                _ => unreachable!(),
            };
            depth[y * 16 + x] = z.to_bits();
        }
    }
    (color, depth)
}
fn gob_word(raw: &[u8], x: usize, y: usize) -> u32 {
    let sector = [
        [0, 2, 16, 18],
        [1, 3, 17, 19],
        [4, 6, 20, 22],
        [5, 7, 21, 23],
        [8, 10, 24, 26],
        [9, 11, 25, 27],
        [12, 14, 28, 30],
        [13, 15, 29, 31],
    ][y][x / 4];
    let at = sector * 16 + (x % 4) * 4;
    u32::from_le_bytes(raw[at..at + 4].try_into().expect("word"))
}
fn set(page: &mut [u8], method: u32, value: u32) {
    let mut at = None;
    for i in (0..WORDS as usize * 4).step_by(8) {
        if u32::from_le_bytes(page[i..i + 4].try_into().unwrap()) & 8191 == method / 4 {
            at = Some(i + 4);
        }
    }
    let at = at.expect("fixture method");
    page[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
pub fn replay(index: u32) -> DepthReplay {
    let mut out = DepthReplay {
        abi_version: 1,
        status: 1,
        draw_count: 0,
        image_bytes: 0,
        background: ClearOp::default(),
        draws: [DrawOp::default(); 3],
        pixels: [0; 512],
        error: [0; 256],
        initial: [0; 128],
        expected: [0; 128],
        functions: [0; 3],
        writes: [0; 3],
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(index, &mut out)))
        .unwrap_or_else(|_| Err("Depth replay panic".into()));
    if let Err(e) = result {
        out.draw_count = 0;
        out.image_bytes = 0;
        for (a, b) in out.error.iter_mut().zip(e.bytes().take(255)) {
            *a = b;
        }
    }
    out
}
fn fill(index: u32, out: &mut DepthReplay) -> Result<(), String> {
    let page = PAGES.get(index as usize).ok_or("Unknown depth case")?;
    let (mem, gpu) = textures::setup(page.to_vec())?;
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &textures::wire(0, WORDS))?;
    check(work.draws().len() == 2 && work.color_clears().len() == 1)?;
    out.background = work.color_clears()[0].into();
    for (i, draw) in work.draws().iter().enumerate() {
        let depth = draw.depth.as_ref().ok_or("Missing guest depth state")?;
        check(
            draw.vertices.len() == 6
                && draw.texture.is_none()
                && draw.indices.is_none()
                && draw.vertex_spirv.len() <= 2048
                && draw.fragment_spirv.len() <= 2048,
        )?;
        let s = draw.surface;
        let d = &mut out.draws[i];
        *d = DrawOp {
            address_upper: (s.address.0 >> 32) as u32,
            address_lower: s.address.0 as u32,
            width: s.width,
            height: s.height,
            pitch: s.pitch,
            format: s.format,
            rect: s.rect,
            mask: s.mask,
            vertex_count: 6,
            vertex_words: draw.vertex_spirv.len() as u32,
            fragment_words: draw.fragment_spirv.len() as u32,
            ..DrawOp::default()
        };
        d.vertex[..draw.vertex_spirv.len()].copy_from_slice(&draw.vertex_spirv);
        d.fragment[..draw.fragment_spirv.len()].copy_from_slice(&draw.fragment_spirv);
        for (a, b) in d.vertices.chunks_exact_mut(4).zip(&draw.vertices) {
            a.copy_from_slice(b);
        }
        out.functions[i] = depth.function;
        out.writes[i] = u32::from(depth.write);
        if i == 0 {
            out.initial.copy_from_slice(&depth.initial);
        }
    }
    work.commit();
    mem.read(0x1800, &mut out.pixels)
        .map_err(|e| e.to_string())?;
    let mut raw = [0; 512];
    mem.read(0x2000, &mut raw).map_err(|e| e.to_string())?;
    for y in 0..8 {
        for x in 0..16 {
            out.expected[y * 16 + x] = gob_word(&raw, x, y);
        }
    }
    check((out.pixels, out.expected) == expected(index as usize))?;
    let mut all = vec![0; 8192];
    mem.read(0x1000, &mut all).map_err(|e| e.to_string())?;
    for range in [0..0x800, 0xa00..0x1000, 0x1200..8192] {
        check(all[range.clone()] == page[range])?;
    }
    out.status = 0;
    out.draw_count = 2;
    out.image_bytes = 512;
    Ok(())
}
fn rejected(page: Vec<u8>, words: u32) -> Result<(), String> {
    let (mem, gpu) = textures::setup(page.clone())?;
    check(
        ChannelProcessor::default()
            .prepare(&gpu, &textures::wire(0, words))
            .is_err(),
    )?;
    let mut actual = vec![0; page.len()];
    mem.read(0x1000, &mut actual).map_err(|e| e.to_string())?;
    check(actual == page)
}
fn core_case(i: usize) -> Result<usize, String> {
    match i {
        0 => {
            for n in 0..9 {
                check(replay(n).status == 0)?;
            }
            Ok(9)
        }
        1 => {
            for (method, value) in [
                (0xfe0, 256),
                (0xfe4, 0x1004),
                (0xfe8, 0x14),
                (0xfec, 16),
                (0xff0, 1),
                (0x1228, 15),
                (0x122c, 7),
                (0x1230, 0),
                (0x7f8, 1),
                (0x1208, 1),
                (0x1538, 0),
                (0x179c, 1),
                (0x12e8, 2),
                (0x130c, 0),
                (0x19bc, 1),
            ] {
                let mut p = PAGES[1].to_vec();
                set(&mut p, method, value);
                rejected(p, WORDS)?;
            }
            Ok(15)
        }
        2 => {
            for value in [
                f32::NAN.to_bits(),
                f32::INFINITY.to_bits(),
                (-0.1f32).to_bits(),
                1.1f32.to_bits(),
            ] {
                let mut p = PAGES[1].to_vec();
                p[0x1000..0x1004].copy_from_slice(&value.to_le_bytes());
                rejected(p, WORDS)?;
            }
            Ok(4)
        }
        3 => {
            for alias in [false, true] {
                let mut p = PAGES[1].to_vec();
                set(&mut p, 0xfe4, 0x800);
                if alias {
                    set(&mut p, 0xfe0, 2);
                }
                let (mem, mut gpu) = textures::setup(p)?;
                if alias {
                    gpu.map(
                        GpuVa(2 << 32),
                        mem.pin(0x1000, 8192, Permissions::READ_WRITE)
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                }
                check(
                    ChannelProcessor::default()
                        .prepare(&gpu, &textures::wire(0, WORDS))
                        .is_err(),
                )?;
                check(textures::pixels(&mem)? == [0; 512])?;
            }
            Ok(2)
        }
        4 => {
            let (mut mem, mut gpu) = textures::setup(PAGES[3].to_vec())?;
            let (_, work) = ChannelProcessor::default().prepare(&gpu, &textures::wire(0, WORDS))?;
            let pin = gpu
                .pin(GpuVa((1 << 32) + 0x800), 512, Access::Read)
                .map_err(|e| e.to_string())?;
            let id = pin.mapping_ids().next().unwrap();
            mem.unmap(0x1000, 8192).map_err(|e| e.to_string())?;
            gpu.unmap(id).map_err(|e| e.to_string())?;
            mem.map(0x1000, vec![99; 8192], Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            work.commit();
            let mut old = [0; 512];
            pin.read(&mut old).map_err(|e| e.to_string())?;
            check(old == expected(3).0)?;
            check(textures::pixels(&mem)? == [99; 512])?;
            Ok(2)
        }
        5 => {
            let mut p = PAGES[3].to_vec();
            p[WORDS as usize * 4..WORDS as usize * 4 + 8]
                .copy_from_slice(&[0x40, 0x02, 0x01, 0x20, 0, 0, 0, 0]);
            rejected(p, WORDS + 2)?;
            let mut p = PAGES[3].to_vec();
            p[0x1308..0x130c].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
            rejected(p, WORDS)?;
            let (mem, gpu) = textures::setup(PAGES[3].to_vec())?;
            let (_, work) = ChannelProcessor::default().prepare(&gpu, &textures::wire(0, WORDS))?;
            drop(work);
            check(textures::pixels(&mem)? == [0; 512])?;
            Ok(3)
        }
        6 => {
            for budget in [1, 4096] {
                let (mem, gpu) = textures::setup(PAGES[1].to_vec())?;
                let mut q = SubmissionQueue::new(&gpu, QueueLimits::default())
                    .map_err(|e| e.to_string())?;
                let id = q
                    .submit(&gpu, &textures::wire(0, WORDS))
                    .map_err(|e| e.to_string())?;
                check(
                    q.prepare_front(
                        &gpu,
                        id,
                        ExecutionLimits {
                            bytes: budget,
                            launches: 64,
                        },
                    )
                    .is_err(),
                )?;
                check(textures::pixels(&mem)? == [0; 512])?;
            }
            Ok(2)
        }
        7 => {
            for (n, page) in PAGES.iter().enumerate().take(8) {
                let mut p = page.to_vec();
                set(&mut p, 0x130c, n as u32 + 1);
                let (mem, gpu) = textures::setup(p)?;
                let (_, work) =
                    ChannelProcessor::default().prepare(&gpu, &textures::wire(0, WORDS))?;
                work.commit();
                check(textures::pixels(&mem)? == expected(n).0)?;
            }
            Ok(8)
        }
        8 => {
            for n in [9, 10] {
                check(replay(n).status == 0)?;
            }
            Ok(2)
        }
        9 => {
            for n in [9, 10] {
                let mut p = PAGES[n].to_vec();
                for base in [0xd00, 0x1300] {
                    for tri in [0, 48] {
                        for byte in 0..16 {
                            p.swap(base + tri + 16 + byte, base + tri + 32 + byte);
                        }
                    }
                }
                let (color, depth) = expected(n);
                check_page(p, color, depth)?;
            }
            let mut p = PAGES[9].to_vec();
            let first = p[0xd00..0xd10].to_vec();
            for vertex in p[0xd00..0xd60].chunks_exact_mut(16) {
                vertex.copy_from_slice(&first);
            }
            let (mut color, mut depth) = expected(8);
            for y in 0..8 {
                for x in 0..16 {
                    if (4..12).contains(&x) && (2..6).contains(&y) {
                        depth[y * 16 + x] = 0.5f32.to_bits();
                    } else {
                        color[(y * 16 + x) * 4..(y * 16 + x + 1) * 4]
                            .copy_from_slice(&[0, 0, 0, 255]);
                    }
                }
            }
            check_page(p, color, depth)?;
            let mut p = PAGES[9].to_vec();
            set(&mut p, 0x12e8, 0);
            set(&mut p, 0x7f8, 1);
            let (color, depth) = expected(8);
            check_page(p, color, depth)?;
            Ok(4)
        }
        _ => Err("Unknown depth check".into()),
    }
}
fn check_page(page: Vec<u8>, color: [u8; 512], depth: [u32; 128]) -> Result<(), String> {
    let (mem, gpu) = textures::setup(page)?;
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &textures::wire(0, WORDS))?;
    work.commit();
    check(textures::pixels(&mem)? == color)?;
    let mut raw = [0; 512];
    mem.read(0x2000, &mut raw).map_err(|e| e.to_string())?;
    for y in 0..8 {
        for x in 0..16 {
            check(gob_word(&raw, x, y) == depth[y * 16 + x])?;
        }
    }
    Ok(())
}
pub fn cases() -> Vec<TextureCase> {
    [
        "depth-all-comparisons-color-and-gob-oracles",
        "depth-invalid-attachment-state",
        "depth-nonfinite-and-out-of-range-storage",
        "depth-color-direct-and-backing-alias-rejection",
        "depth-capture-unmap-and-address-reuse",
        "depth-late-failure-and-cancel-rollback",
        "depth-shared-memory-budget-rollback",
        "depth-d3d-and-ogl-comparison-encodings",
        "depth-sloped-forward-and-reverse-oracles",
        "depth-sloped-winding-degenerate-and-write-control",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        let r = core_case(i);
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
    fn depth_cases_and_value_layout() {
        assert_eq!(std::mem::size_of::<super::DepthReplay>(), 51500);
        for c in super::cases() {
            assert!(c.passed, "{}: {}", c.name, c.error);
        }
        assert_ne!(super::replay(11).status, 0);
    }
}
