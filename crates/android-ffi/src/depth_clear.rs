//! Original depth-clear render corpus and portable transactional test matrix.
use crate::{
    depth::DepthReplay,
    draws::DrawOp,
    graphics::ClearOp,
    textures::{self, TextureCase},
};
use emulator_gpu::{
    ChannelProcessor, ExecutionLimits, GpuAddressSpace, GpuVa, PreparedSubmission, QueueLimits,
    SubmissionQueue,
};
use emulator_memory::{Access, AddressSpace, Permissions};
const BASE: u64 = 1 << 32;
const PAGES: [&[u8]; 16] = [
    include_bytes!("../../../examples/gpu/depth-clear-0-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-1-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-2-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-3-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-4-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-5-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-6-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-7-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-8-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-9-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-10-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-11-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-12-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-13-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-14-page.bin"),
    include_bytes!("../../../examples/gpu/depth-clear-15-page.bin"),
];
const WORDS: [u32; 16] = [
    184, 184, 184, 190, 184, 206, 184, 184, 184, 184, 184, 184, 184, 184, 184, 206,
];
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Clear {
    pub rect: [u32; 4],
    pub value: u32,
    pub before_draw: u32,
}
#[repr(C)]
pub struct ClearReplay {
    pub scene: DepthReplay,
    pub original: [u32; 128],
    pub count: u32,
    pub clears: [Clear; 4],
}
fn check(v: bool) -> Result<(), String> {
    if v {
        Ok(())
    } else {
        Err("Depth-clear check failed".into())
    }
}
fn expected(i: usize) -> ([u8; 512], [u32; 128]) {
    let mut color = [0; 512];
    let mut depth = [0; 128];
    for y in 0..8 {
        for x in 0..16 {
            let center = (4..12).contains(&x) && (2..6).contains(&y);
            let region = match i {
                2 | 5 => x >= 8,
                3 => (8..14).contains(&x) && (1..7).contains(&y),
                14 => x == 15 && y == 7,
                _ => true,
            };
            let black = matches!(i, 1 | 11) || !region;
            let red = center && (matches!(i, 6 | 10 | 13) || i == 9 && x >= 8);
            let pixel = if black {
                [0, 0, 0, 255]
            } else if red {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            };
            let z: f32 = match i {
                0 | 4 => 0.25,
                1 | 7 => 0.,
                2 | 3 | 5 | 14 => {
                    if region {
                        0.25
                    } else {
                        0.
                    }
                }
                6 => {
                    if center {
                        0.75
                    } else {
                        1.
                    }
                }
                8 | 11 | 13 => 0.5,
                9 => {
                    if center && x >= 8 {
                        0.75
                    } else if x >= 8 {
                        1.
                    } else {
                        0.25
                    }
                }
                10 => 1.,
                12 => 0.75,
                15 => {
                    if center {
                        0.875
                    } else {
                        0.25
                    }
                }
                _ => unreachable!(),
            };
            color[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].copy_from_slice(&pixel);
            depth[y * 16 + x] = z.to_bits();
        }
    }
    (color, depth)
}
fn word(raw: &[u8], x: usize, y: usize) -> u32 {
    let sectors = [
        [0, 2, 16, 18],
        [1, 3, 17, 19],
        [4, 6, 20, 22],
        [5, 7, 21, 23],
        [8, 10, 24, 26],
        [9, 11, 25, 27],
        [12, 14, 28, 30],
        [13, 15, 29, 31],
    ];
    let at = sectors[y][x / 4] * 16 + x % 4 * 4;
    u32::from_le_bytes(raw[at..at + 4].try_into().expect("word"))
}
fn result(mem: &AddressSpace) -> Result<([u8; 512], [u32; 128]), String> {
    let c = textures::pixels(mem)?;
    let mut raw = [0; 512];
    mem.read(0x2000, &mut raw).map_err(|e| e.to_string())?;
    let d = std::array::from_fn(|i| word(&raw, i % 16, i / 16));
    Ok((c, d))
}
fn empty() -> DepthReplay {
    DepthReplay {
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
    }
}
pub fn replay(index: u32) -> ClearReplay {
    let mut out = ClearReplay {
        scene: empty(),
        original: [0; 128],
        count: 0,
        clears: [Clear::default(); 4],
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(index, &mut out)))
        .unwrap_or_else(|_| Err("Depth-clear replay panic".into()));
    if let Err(e) = r {
        out.scene.status = 1;
        out.scene.draw_count = 0;
        out.scene.image_bytes = 0;
        out.count = 0;
        for (a, b) in out.scene.error.iter_mut().zip(e.bytes().take(255)) {
            *a = b;
        }
    }
    out
}
fn fill(index: u32, out: &mut ClearReplay) -> Result<(), String> {
    let page = *PAGES
        .get(index as usize)
        .ok_or("Unknown depth-clear case")?;
    let (mem, gpu) = textures::setup(page.to_vec())?;
    let (_, work) =
        ChannelProcessor::default().prepare(&gpu, &textures::wire(0, WORDS[index as usize]))?;
    check(
        work.draws().len() == 2
            && work.color_clears().len() == 1
            && !work.depth_clears().is_empty()
            && work.depth_clears().len() <= 4,
    )?;
    out.scene.background = work.color_clears()[0].into();
    for (i, d) in work.draws().iter().enumerate() {
        let z = d.depth.as_ref().ok_or("Missing depth state")?;
        check(
            z.address == GpuVa(BASE + 0x1000)
                && d.vertices.len() == 6
                && d.texture.is_none()
                && d.indices.is_none()
                && d.vertex_spirv.len() <= 2048
                && d.fragment_spirv.len() <= 2048,
        )?;
        let s = d.surface;
        let p = &mut out.scene.draws[i];
        *p = DrawOp {
            address_upper: (s.address.0 >> 32) as u32,
            address_lower: s.address.0 as u32,
            width: s.width,
            height: s.height,
            pitch: s.pitch,
            format: s.format,
            rect: s.rect,
            mask: s.mask,
            vertex_count: 6,
            vertex_words: d.vertex_spirv.len() as u32,
            fragment_words: d.fragment_spirv.len() as u32,
            ..DrawOp::default()
        };
        p.vertex[..d.vertex_spirv.len()].copy_from_slice(&d.vertex_spirv);
        p.fragment[..d.fragment_spirv.len()].copy_from_slice(&d.fragment_spirv);
        for (a, b) in p.vertices.chunks_exact_mut(4).zip(&d.vertices) {
            a.copy_from_slice(b);
        }
        out.scene.functions[i] = z.function;
        out.scene.writes[i] = u32::from(z.write);
        if i == 0 {
            out.scene.initial.copy_from_slice(&z.initial);
        }
    }
    for (i, c) in work.depth_clears().iter().enumerate() {
        check(
            c.address == GpuVa(BASE + 0x1000)
                && c.width == 16
                && c.height == 8
                && c.before_draw <= 2,
        )?;
        out.clears[i] = Clear {
            rect: c.rect,
            value: c.value,
            before_draw: c.before_draw,
        };
    }
    out.count = work.depth_clears().len() as u32;
    out.original = std::array::from_fn(|i| word(&page[0x1000..0x1200], i % 16, i / 16));
    work.commit();
    let (c, d) = result(&mem)?;
    check((c, d) == expected(index as usize))?;
    out.scene.pixels = c;
    out.scene.expected = d;
    let mut actual = vec![0; 8192];
    mem.read(0x1000, &mut actual).map_err(|e| e.to_string())?;
    for r in [0..0x800, 0xa00..0x1000, 0x1200..8192] {
        check(actual[r.clone()] == page[r])?;
    }
    out.scene.status = 0;
    out.scene.draw_count = 2;
    out.scene.image_bytes = 512;
    Ok(())
}
fn only(value: u32, rect: [u32; 4], control: u32) -> Vec<(u32, u32)> {
    vec![
        (0, 0xb197),
        (0xfe0, 1),
        (0xfe4, 0x1000),
        (0xfe8, 0xa),
        (0xfec, 0),
        (0xff0, 0),
        (0x1228, 16),
        (0x122c, 8),
        (0x1230, 1),
        (0x7f8, 0),
        (0x1208, 0),
        (0x1538, 1),
        (0x179c, 0),
        (0x19bc, 0),
        (0xff4, 16 << 16),
        (0xff8, 8 << 16),
        (0x10f8, control),
        (0xd6c, rect[0] | rect[2] << 16),
        (0xd70, rect[1] | rect[3] << 16),
        (0xe00, 1),
        (0xe04, 8 << 16 | 4),
        (0xe08, 6 << 16 | 2),
        (0xd90, value),
        (0x19d0, 1),
    ]
}
fn page(methods: &[(u32, u32)]) -> Vec<u8> {
    let mut p = vec![0; 8192];
    let b: Vec<u8> = methods
        .iter()
        .flat_map(|(m, v)| [0x20010000 | (m / 4), *v])
        .flat_map(u32::to_le_bytes)
        .collect();
    p[..b.len()].copy_from_slice(&b);
    for w in p[0x1000..0x1200].chunks_exact_mut(4) {
        w.copy_from_slice(&0xaaaa5555u32.to_le_bytes());
    }
    p
}
fn change(p: &mut [u8], words: u32, m: u32, v: u32) {
    let at = (0..words as usize * 4)
        .step_by(8)
        .rfind(|&i| u32::from_le_bytes(p[i..i + 4].try_into().unwrap()) & 8191 == m / 4)
        .expect("method");
    p[at + 4..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn run(
    p: Vec<u8>,
    words: u32,
) -> Result<(AddressSpace, GpuAddressSpace, PreparedSubmission), String> {
    let (mem, gpu) = textures::setup(p)?;
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &textures::wire(0, words))?;
    Ok((mem, gpu, work))
}
fn rejected(p: Vec<u8>, words: u32) -> Result<(), String> {
    let (mem, gpu) = textures::setup(p.clone())?;
    check(
        ChannelProcessor::default()
            .prepare(&gpu, &textures::wire(0, words))
            .is_err(),
    )?;
    let mut a = vec![0; 8192];
    mem.read(0x1000, &mut a).map_err(|e| e.to_string())?;
    check(a == p)
}
fn permissions(
    mem: &AddressSpace,
    gpu: &mut GpuAddressSpace,
    rights: Option<Permissions>,
) -> Result<(), String> {
    let id = gpu
        .pin(GpuVa(BASE), 1, Access::Read)
        .map_err(|e| e.to_string())?
        .mapping_ids()
        .next()
        .unwrap();
    gpu.unmap(id).map_err(|e| e.to_string())?;
    for (a, c, len, r) in [
        (BASE, 0x1000, 4096, Permissions::READ_WRITE),
        (BASE + 0x1200, 0x2200, 3584, Permissions::READ_WRITE),
    ] {
        gpu.map(GpuVa(a), mem.pin(c, len, r).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    if let Some(r) = rights {
        gpu.map(
            GpuVa(BASE + 0x1000),
            mem.pin(0x2000, 512, r).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn core(i: usize) -> Result<usize, String> {
    match i {
        0 => {
            for n in 0..16 {
                check(replay(n).scene.status == 0)?;
            }
            Ok(16)
        }
        1 => {
            for v in [0f32, 0.25, 0.5, 0.75, 1.] {
                let m = only(v.to_bits(), [0, 0, 16, 8], 0);
                let (mem, _, w) = run(page(&m), m.len() as u32 * 2)?;
                w.commit();
                check(result(&mem)?.1 == [v.to_bits(); 128])?;
            }
            Ok(5)
        }
        2 => {
            for r in [
                [0, 0, 1, 1],
                [15, 7, 16, 8],
                [4, 2, 12, 6],
                [0, 0, 16, 1],
                [0, 7, 16, 8],
                [0, 0, 1, 8],
                [15, 0, 16, 8],
                [1, 1, 15, 7],
                [0, 0, 16, 8],
            ] {
                let m = only(0.5f32.to_bits(), r, 0x10);
                let (mem, _, w) = run(page(&m), m.len() as u32 * 2)?;
                w.commit();
                let d = result(&mem)?.1;
                for y in 0..8 {
                    for x in 0..16 {
                        check(
                            d[y * 16 + x]
                                == if x >= r[0] as usize
                                    && x < (r[2] as usize)
                                    && y >= r[1] as usize
                                    && y < (r[3] as usize)
                                {
                                    0.5f32.to_bits()
                                } else {
                                    0xaaaa5555
                                },
                        )?;
                    }
                }
            }
            Ok(9)
        }
        3 => {
            for control in [0x100, 0x110] {
                let m = only(0.75f32.to_bits(), [0, 0, 6, 8], control);
                let (mem, _, w) = run(page(&m), m.len() as u32 * 2)?;
                w.commit();
                let d = result(&mem)?.1;
                for y in 0..8 {
                    for x in 0..16 {
                        let inside = (2..6).contains(&y)
                            && (4..if control == 0x100 { 8 } else { 6 }).contains(&x);
                        check(
                            d[y * 16 + x]
                                == if inside {
                                    0.75f32.to_bits()
                                } else {
                                    0xaaaa5555
                                },
                        )?;
                    }
                }
            }
            Ok(2)
        }
        4 => {
            for r in [[16, 0, 16, 8], [20, 0, 24, 8], [0, 8, 16, 12]] {
                let m = only(0, [r[0], r[1], r[2], r[3]], 0x10);
                let (mem, mut gpu) = textures::setup(page(&m))?;
                permissions(&mem, &mut gpu, None)?;
                let (_, w) = ChannelProcessor::default()
                    .prepare(&gpu, &textures::wire(0, m.len() as u32 * 2))?;
                check(w.write_ranges().count() == 0 && w.stats().cleared_bytes == 0)?;
                w.commit();
                check(result(&mem)?.1 == [0xaaaa5555; 128])?;
            }
            Ok(3)
        }
        5 => {
            let mut m = only(0, [0, 0, 16, 8], 0);
            m.iter_mut().find(|v| v.0 == 0x7f8).unwrap().1 = 1;
            rejected(page(&m), m.len() as u32 * 2)?;
            Ok(1)
        }
        6 => {
            for flags in [0, 2, 3, 0x3d, 0x41, 0x10001] {
                let mut m = only(0, [0, 0, 16, 8], 0);
                m.last_mut().unwrap().1 = flags;
                rejected(page(&m), m.len() as u32 * 2)?;
            }
            Ok(6)
        }
        7 => {
            for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1, 1.1, -0.0] {
                let m = only(v.to_bits(), [0, 0, 16, 8], 0);
                rejected(page(&m), m.len() as u32 * 2)?;
            }
            Ok(6)
        }
        8 => {
            for field in [
                0xfe0, 0xfe4, 0xfe8, 0xfec, 0xff0, 0x1228, 0x122c, 0x1230, 0x7f8, 0x1208, 0x1538,
                0x179c, 0x19bc, 0xff4, 0xff8, 0x10f8, 0xd90,
            ] {
                let mut m = only(0, [0, 0, 16, 8], 0);
                m.iter_mut().find(|v| v.0 == field).unwrap().0 = 0x100;
                rejected(page(&m), m.len() as u32 * 2)?;
            }
            Ok(17)
        }
        9 => {
            for (field, v) in [
                (0xfe0, 256),
                (0xfe4, 0x1004),
                (0xfe8, 0x14),
                (0xfec, 16),
                (0xff0, 1),
                (0x1228, 0),
                (0x1228, 17),
                (0x122c, 9),
                (0x1230, 0),
                (0x1208, 1),
                (0x1538, 0),
                (0x179c, 1),
                (0x19bc, 1),
                (0x10f8, 1),
            ] {
                let mut m = only(0, [0, 0, 16, 8], 0);
                m.iter_mut().find(|a| a.0 == field).unwrap().1 = v;
                rejected(page(&m), m.len() as u32 * 2)?;
            }
            Ok(14)
        }
        10 => {
            let m = only(0.25f32.to_bits(), [0, 0, 16, 8], 0);
            let (mem, mut gpu) = textures::setup(page(&m))?;
            permissions(&mem, &mut gpu, Some(Permissions::WRITE))?;
            let (_, w) = ChannelProcessor::default()
                .prepare(&gpu, &textures::wire(0, m.len() as u32 * 2))?;
            w.commit();
            check(result(&mem)?.1 == [0.25f32.to_bits(); 128])?;
            Ok(1)
        }
        11 => {
            for rights in [
                None,
                Some(Permissions::READ),
                Some(Permissions::WRITE),
                Some(Permissions::NONE),
            ] {
                let m = only(0, [4, 2, 12, 6], 0x10);
                let (mem, mut gpu) = textures::setup(page(&m))?;
                permissions(&mem, &mut gpu, rights)?;
                check(
                    ChannelProcessor::default()
                        .prepare(&gpu, &textures::wire(0, m.len() as u32 * 2))
                        .is_err(),
                )?;
                check(result(&mem)?.1 == [0xaaaa5555; 128])?;
            }
            let m = only(0, [4, 2, 12, 6], 0x10);
            let (mem, mut gpu) = textures::setup(page(&m))?;
            permissions(&mem, &mut gpu, None)?;
            gpu.map(
                GpuVa(BASE + 0x1000),
                mem.pin(0x2000, 511, Permissions::READ_WRITE)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &textures::wire(0, m.len() as u32 * 2))
                    .is_err(),
            )?;
            check(result(&mem)?.1 == [0xaaaa5555; 128])?;
            Ok(5)
        }
        12 => {
            for alias in [false, true] {
                let mut p = PAGES[0].to_vec();
                change(&mut p, WORDS[0], 0xfe4, 0x800);
                if alias {
                    change(&mut p, WORDS[0], 0xfe0, 2);
                }
                let (mem, mut gpu) = textures::setup(p.clone())?;
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
                        .prepare(&gpu, &textures::wire(0, WORDS[0]))
                        .is_err(),
                )?;
                check(textures::pixels(&mem)? == [0; 512])?;
            }
            Ok(2)
        }
        13 => {
            for addr in [0xa00, 0xc00] {
                let mut p = PAGES[0].to_vec();
                change(&mut p, WORDS[0], 0xfe4, addr);
                rejected(p, WORDS[0])?;
            }
            let mut p = PAGES[0].to_vec();
            change(&mut p, WORDS[0], 0xfe0, 2);
            change(&mut p, WORDS[0], 0xfe4, 0);
            let (mem, mut gpu) = textures::setup(p)?;
            gpu.map(
                GpuVa(2 << 32),
                mem.pin(0x1d00, 512, Permissions::READ_WRITE)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &textures::wire(0, WORDS[0]))
                    .is_err(),
            )?;
            check(textures::pixels(&mem)? == [0; 512])?;
            Ok(3)
        }
        14 => {
            let mut p = PAGES[6].to_vec();
            let at = WORDS[6] as usize * 4;
            p[at..at + 8].copy_from_slice(&[0x40, 2, 1, 0x20, 0, 0, 0, 0]);
            rejected(p, WORDS[6] + 2)?;
            let mut m = only(0, [0, 0, 16, 8], 0);
            m.extend([(0xfe4, 0x3e00), (0x19d0, 1)]);
            rejected(page(&m), m.len() as u32 * 2)?;
            Ok(2)
        }
        15 => {
            let (mem, _, w) = run(PAGES[9].to_vec(), WORDS[9])?;
            drop(w);
            check(textures::pixels(&mem)? == [0; 512])?;
            let (mut mem, mut gpu, w) = run(PAGES[9].to_vec(), WORDS[9])?;
            let pin = gpu
                .pin(GpuVa(BASE), 8192, Access::Read)
                .map_err(|e| e.to_string())?;
            let id = pin.mapping_ids().next().unwrap();
            gpu.unmap(id).map_err(|e| e.to_string())?;
            mem.unmap(0x1000, 8192).map_err(|e| e.to_string())?;
            mem.map(0x1000, vec![99; 8192], Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            w.commit();
            let mut old = vec![0; 8192];
            pin.read(&mut old).map_err(|e| e.to_string())?;
            check(old[0x800..0xa00] == expected(9).0)?;
            check(textures::pixels(&mem)? == [99; 512])?;
            drop(pin);
            check(mem.resident_bytes() == 8192)?;
            Ok(3)
        }
        16 => {
            for (full, budget) in [(true, 512), (false, 1024)] {
                let m = only(0, [4, 2, 12, 6], if full { 0 } else { 0x10 });
                let (mem, gpu) = textures::setup(page(&m))?;
                for b in [budget - 1, budget] {
                    let mut q = SubmissionQueue::new(&gpu, QueueLimits::default())
                        .map_err(|e| e.to_string())?;
                    let id = q
                        .submit(&gpu, &textures::wire(0, m.len() as u32 * 2))
                        .map_err(|e| e.to_string())?;
                    let r = q.prepare_front(
                        &gpu,
                        id,
                        ExecutionLimits {
                            bytes: b,
                            launches: 1,
                        },
                    );
                    check(r.is_ok() == (b == budget))?;
                    drop(r);
                    check(result(&mem)?.1 == [0xaaaa5555; 128])?;
                }
            }
            let mut m = only(0, [0, 0, 16, 8], 0);
            m.push((0x19d0, 1));
            let (mem, gpu) = textures::setup(page(&m))?;
            let mut q =
                SubmissionQueue::new(&gpu, QueueLimits::default()).map_err(|e| e.to_string())?;
            let id = q
                .submit(&gpu, &textures::wire(0, m.len() as u32 * 2))
                .map_err(|e| e.to_string())?;
            check(
                q.prepare_front(
                    &gpu,
                    id,
                    ExecutionLimits {
                        bytes: 4096,
                        launches: 1,
                    },
                )
                .is_err(),
            )?;
            check(result(&mem)?.1 == [0xaaaa5555; 128])?;
            let mut m = only(0, [0, 0, 16, 8], 0);
            m.extend(std::iter::repeat_n((0x19d0, 1), 64));
            rejected(page(&m), m.len() as u32 * 2)?;
            let mut m = only(0, [0, 0, 16, 8], 0);
            m.extend(std::iter::repeat_n((0x19d0, 1), 8));
            let (mem, mut gpu) = textures::setup(page(&m))?;
            permissions(&mem, &mut gpu, None)?;
            for i in 0..512 {
                gpu.map(
                    GpuVa(BASE + 0x1000 + i),
                    mem.pin(0x2000 + i, 1, Permissions::READ_WRITE)
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            }
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &textures::wire(0, m.len() as u32 * 2))
                    .is_err(),
            )?;
            check(result(&mem)?.1 == [0xaaaa5555; 128])?;
            Ok(7)
        }
        17 => {
            let m = only(0.25f32.to_bits(), [0, 0, 16, 8], 0);
            let (mut mem, gpu) = textures::setup(page(&m))?;
            let (mut next, w) = ChannelProcessor::default()
                .prepare(&gpu, &textures::wire(0, m.len() as u32 * 2))?;
            w.commit();
            for value in [0.75f32, 0.5] {
                let ops = [(0xd90, value.to_bits()), (0x19d0, 1)];
                let p = page(&ops);
                mem.write(0x1000, &p[..16]).map_err(|e| e.to_string())?;
                let (n, w) = next.prepare(&gpu, &textures::wire(0, 4))?;
                w.commit();
                next = n;
                check(result(&mem)?.1 == [value.to_bits(); 128])?;
            }
            Ok(3)
        }
        18 => {
            let truth = [
                [false, false, false],
                [true, false, false],
                [false, true, false],
                [true, true, false],
                [false, false, true],
                [true, false, true],
                [false, true, true],
                [true, true, true],
            ];
            for (kind, row) in truth.iter().enumerate() {
                for (delta, &passes) in row.iter().enumerate() {
                    let mut p = PAGES[0].to_vec();
                    change(&mut p, WORDS[0], 0x130c, 0x200 + kind as u32);
                    change(&mut p, WORDS[0], 0xd90, 0.5f32.to_bits());
                    let z = 0.5f32.to_bits() + delta as u32 - 1;
                    for v in p[0xd00..0xd60].chunks_exact_mut(16) {
                        v[8..12].copy_from_slice(&z.to_le_bytes());
                    }
                    let end = (0..WORDS[0] as usize * 4)
                        .step_by(8)
                        .find(|&at| {
                            u32::from_le_bytes(p[at..at + 4].try_into().unwrap()) & 8191
                                == 0x1214 / 4
                        })
                        .unwrap()
                        + 8;
                    let (mem, _, w) = run(p, end as u32 / 4)?;
                    w.commit();
                    let (c, d) = result(&mem)?;
                    check(c.chunks_exact(4).all(|p| {
                        p == if passes {
                            [0, 255, 0, 255]
                        } else {
                            [0, 0, 0, 255]
                        }
                    }))?;
                    check(d == [if passes { z } else { 0.5f32.to_bits() }; 128])?;
                }
            }
            Ok(24)
        }
        19 => {
            let m = only(0, [0, 0, 16, 8], 0);
            let (mut mem, gpu) = textures::setup(page(&m))?;
            let (mut next, w) = ChannelProcessor::default()
                .prepare(&gpu, &textures::wire(0, m.len() as u32 * 2))?;
            w.commit();
            for i in 0..128 {
                let value = if i % 2 == 0 { 0.25f32 } else { 0.75 };
                let p = page(&[(0xd90, value.to_bits()), (0x19d0, 1)]);
                mem.write(0x1000, &p[..16]).map_err(|e| e.to_string())?;
                let (n, w) = next.prepare(&gpu, &textures::wire(0, 4))?;
                let (q, s) = w.commit();
                check(
                    q.stats().pending == 0 && s.clear_launches == 1 && mem.resident_bytes() == 8192,
                )?;
                next = n;
                check(result(&mem)?.1 == [value.to_bits(); 128])?;
            }
            drop(gpu);
            mem.unmap(0x1000, 8192).map_err(|e| e.to_string())?;
            check(mem.resident_bytes() == 0)?;
            Ok(128)
        }
        _ => Err("Unknown depth-clear test".into()),
    }
}
const NAMES: [&str; 20] = [
    "zclear-sixteen-color-and-depth-oracles",
    "zclear-zero-one-and-dyadic-values",
    "zclear-nine-rectangles-preserve-other-bits",
    "zclear-scissor-and-rectangle-intersection",
    "zclear-empty-unmapped-target-no-effect",
    "zclear-read-only-target-state-rejection",
    "zclear-stencil-mixed-and-reserved-flags",
    "zclear-nonfinite-out-of-range-and-negative-zero",
    "zclear-seventeen-uninitialized-fields",
    "zclear-fourteen-invalid-attachment-states",
    "zclear-write-only-full-clear-no-color-target",
    "zclear-permission-and-hole-rollback",
    "zclear-direct-and-backing-color-feedback",
    "zclear-shader-and-vertex-backing-feedback",
    "zclear-late-method-and-hole-rollback",
    "zclear-cancel-unmap-reuse-and-final-release",
    "zclear-byte-launch-and-span-budget-rollback",
    "zclear-retained-state-across-submissions",
    "zclear-eight-comparisons-at-three-ulp-boundaries",
    "zclear-128-retained-state-resource-cycles",
];
pub fn cases() -> Vec<TextureCase> {
    NAMES
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let r = std::panic::catch_unwind(|| core(i))
                .unwrap_or_else(|_| Err("Depth-clear test panic".into()));
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
    macro_rules! cases{($($name:ident=$i:expr),*)=>{$(#[test]fn $name(){super::core($i).unwrap();})*}}
    cases!(
        images = 0,
        values = 1,
        rectangles = 2,
        scissor = 3,
        empty = 4,
        readonly = 5,
        flags = 6,
        invalid_values = 7,
        missing_fields = 8,
        invalid_state = 9,
        write_only = 10,
        permissions = 11,
        color_alias = 12,
        shader_alias = 13,
        late_failure = 14,
        lifetime = 15,
        budgets = 16,
        retained = 17,
        boundaries = 18,
        cycles = 19
    );
    #[test]
    fn abi_and_invalid_index() {
        assert_eq!(std::mem::size_of::<super::ClearReplay>(), 52112);
        assert_eq!(std::mem::align_of::<super::ClearReplay>(), 4);
        let r = super::replay(16);
        assert_eq!((r.scene.status, r.scene.draw_count, r.count), (1, 0, 0));
    }
}
