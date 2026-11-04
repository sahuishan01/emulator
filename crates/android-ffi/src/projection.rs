//! Positive-W owned corpus; rectangle/texel/depth formulas do not use the rasterizer.
use crate::{
    depth::DepthReplay,
    draws::DrawOp,
    indexed::IndexedReplay,
    textures::{self, check, TextureCase, TextureReplay},
};
use emulator_gpu::{
    ChannelProcessor, ExecutionLimits, GpuVa, PreparedSubmission, QueueLimits, SubmissionQueue,
};
use emulator_memory::{Access, AddressSpace, Permissions};
const PAGES: [&[u8]; 16] = [
    include_bytes!("../../../examples/gpu/projection-0-page.bin"),
    include_bytes!("../../../examples/gpu/projection-1-page.bin"),
    include_bytes!("../../../examples/gpu/projection-2-page.bin"),
    include_bytes!("../../../examples/gpu/projection-3-page.bin"),
    include_bytes!("../../../examples/gpu/projection-4-page.bin"),
    include_bytes!("../../../examples/gpu/projection-5-page.bin"),
    include_bytes!("../../../examples/gpu/projection-6-page.bin"),
    include_bytes!("../../../examples/gpu/projection-7-page.bin"),
    include_bytes!("../../../examples/gpu/projection-8-page.bin"),
    include_bytes!("../../../examples/gpu/projection-9-page.bin"),
    include_bytes!("../../../examples/gpu/projection-10-page.bin"),
    include_bytes!("../../../examples/gpu/projection-11-page.bin"),
    include_bytes!("../../../examples/gpu/projection-12-page.bin"),
    include_bytes!("../../../examples/gpu/projection-13-page.bin"),
    include_bytes!("../../../examples/gpu/projection-14-page.bin"),
    include_bytes!("../../../examples/gpu/projection-15-page.bin"),
];
const RECTS: [[f64; 4]; 12] = [
    [4., 2., 12., 6.],
    [0., 0., 16., 8.],
    [0., 0., 16., 8.],
    [2., 1., 14., 7.],
    [0., 0., 16., 8.],
    [0., 0., 16., 8.],
    [2., 0., 14., 8.],
    [0.125, 0.125, 15.875, 7.875],
    [0., 0., 16., 8.],
    [2., 0., 14., 8.],
    [0., 0., 16., 8.],
    [4., 0., 12., 8.],
];
fn words(i: usize) -> u32 {
    if i < 8 {
        142
    } else if i < 12 {
        154
    } else {
        162
    }
}
fn plan(i: usize) -> Result<(AddressSpace, PreparedSubmission), String> {
    let (m, g) = textures::setup(PAGES[i].to_vec())?;
    let (_, w) = ChannelProcessor::default().prepare(&g, &textures::wire(0, words(i)))?;
    Ok((m, w))
}
fn center(r: [f64; 4]) -> [f64; 4] {
    [
        (r[0] + 8.) / 2.,
        (r[1] + 4.) / 2.,
        (r[2] + 8.) / 2.,
        (r[3] + 4.) / 2.,
    ]
}
fn inside(r: [f64; 4], x: f64, y: f64) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}
pub(crate) fn expected(i: usize) -> [u8; 512] {
    let mut out = [0; 512];
    for y in 0..8 {
        for x in 0..16 {
            let p = if i < 12 {
                let xf = x as f64 + 0.5;
                let yf = y as f64 + 0.5;
                let rect = RECTS[i];
                let c = center(rect);
                let clipped = (i == 5 || i == 11) && x < 8;
                if clipped || !inside(rect, xf, yf) {
                    [0, 0, 0, 255]
                } else {
                    let second = inside(c, xf, yf) && (i != 10 || x + 2 * y <= 14);
                    let r = if second { c } else { rect };
                    let u = (1.5 * (xf - r[0]) / (r[2] - r[0]) - 0.25) * 4.;
                    let v = (1.5 * (yf - r[1]) / (r[3] - r[1]) - 0.25) * 4.;
                    let tx = (u.floor() as i32).clamp(0, 3);
                    let ty = (v.floor() as i32).clamp(0, 3);
                    let mut p = [
                        [255, 0, 0, 255],
                        [255, 255, 0, 255],
                        [0, 255, 0, 255],
                        [0, 255, 255, 255],
                    ][((tx + ty + i32::from(i == 10)) % 4) as usize];
                    if i == 4 {
                        p.swap(0, 2);
                    }
                    p
                }
            } else {
                let c = (4..12).contains(&x) && (2..6).contains(&y);
                if i == 15 && !c {
                    [0, 0, 0, 255]
                } else if c && (i == 14 || i < 14 && x >= 8) {
                    [255, 0, 0, 255]
                } else {
                    [0, 255, 0, 255]
                }
            };
            out[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].copy_from_slice(&p);
        }
    }
    out
}
fn depth_expected(i: usize) -> [u32; 128] {
    std::array::from_fn(|p| {
        let x = p % 16;
        let y = p / 16;
        let c = (4..12).contains(&x) && (2..6).contains(&y);
        let z = if i == 14 {
            1.
        } else if i == 15 {
            if c {
                0.125
            } else {
                1.
            }
        } else if c && x >= 8 {
            0.5
        } else {
            let z = (2 * x + 1) as f32 / 32.;
            if i == 12 {
                z
            } else {
                1. - z
            }
        };
        z.to_bits()
    })
}
fn verify(i: usize, m: &AddressSpace, w: PreparedSubmission) -> Result<[u8; 512], String> {
    check(w.draws().len() == 2 && w.color_clears().len() == 1 && w.depth_clears().is_empty())?;
    let stats = w.commit().1;
    check(stats.draw_launches == 2 && stats.clear_launches == 1)?;
    let result = textures::pixels(m)?;
    check(result == expected(i))?;
    // Check all command/input bytes unchanged. Depth attachment has its own oracle.
    let mut raw = vec![0; PAGES[i].len()];
    m.read(0x1000, &mut raw).map_err(|e| e.to_string())?;
    check(raw[..0x800] == PAGES[i][..0x800] && raw[0xa00..0x1000] == PAGES[i][0xa00..0x1000])?;
    if i >= 12 {
        // Independently invert the 16-byte sector table; do not call GPU GOB code.
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
        let want = depth_expected(i);
        for y in 0..8 {
            for x in 0..16 {
                let at = 0x1000 + sectors[y][x / 4] * 16 + (x % 4) * 4;
                check(raw[at..at + 4] == want[y * 16 + x].to_le_bytes())?;
            }
        }
        check(raw[0x1200..] == PAGES[i][0x1200..])?;
    }
    Ok(result)
}
pub(crate) fn fill_texture(i: u32, out: &mut TextureReplay, normalize: bool) -> Result<(), String> {
    let i = i as usize;
    if i >= 8 {
        return Err("Unknown projection texture case".into());
    }
    let (m, w) = plan(i)?;
    out.background = w.color_clears()[0].into();
    for (d, s) in out.draws.iter_mut().zip(w.draws()) {
        *d = textures::attributes(s)?;
        if normalize {
            textures::normalized_vertex(s, &mut d.draw)?;
        }
    }
    out.pixels = verify(i, &m, w)?;
    out.status = 0;
    out.draw_count = 2;
    out.image_bytes = 512;
    Ok(())
}
pub(crate) fn fill_indexed(i: u32, out: &mut IndexedReplay) -> Result<(), String> {
    let i = i as usize + 8;
    if i >= 12 {
        return Err("Unknown projection indexed case".into());
    }
    let (m, w) = plan(i)?;
    out.background = w.color_clears()[0].into();
    for (d, s) in out.draws.iter_mut().zip(w.draws()) {
        d.textured = textures::attributes(s)?;
        let idx = s.indices.as_ref().ok_or("Missing projection indices")?;
        check(idx.values.len() <= 96)?;
        d.index_count = idx.values.len() as u32;
        d.element_bytes = idx.element_bytes;
        d.base_vertex = idx.base_vertex;
        d.indices[..idx.values.len()].copy_from_slice(&idx.values);
    }
    out.pixels = verify(i, &m, w)?;
    out.status = 0;
    out.draw_count = 2;
    out.image_bytes = 512;
    Ok(())
}
pub(crate) fn fill_depth(i: u32, out: &mut DepthReplay) -> Result<(), String> {
    let i = i as usize + 12;
    if i >= 16 {
        return Err("Unknown projection depth case".into());
    }
    let (m, w) = plan(i)?;
    out.background = w.color_clears()[0].into();
    for (n, s) in w.draws().iter().enumerate() {
        let z = s.depth.as_ref().ok_or("Missing projected depth")?;
        check(
            s.vertices.len() == 6 && s.vertex_spirv.len() <= 2048 && s.fragment_spirv.len() <= 2048,
        )?;
        let t = s.surface;
        let d = &mut out.draws[n];
        *d = DrawOp {
            address_upper: (t.address.0 >> 32) as u32,
            address_lower: t.address.0 as u32,
            width: t.width,
            height: t.height,
            pitch: t.pitch,
            format: t.format,
            rect: t.rect,
            mask: t.mask,
            vertex_count: 6,
            vertex_words: s.vertex_spirv.len() as u32,
            fragment_words: s.fragment_spirv.len() as u32,
            ..DrawOp::default()
        };
        d.vertex[..s.vertex_spirv.len()].copy_from_slice(&s.vertex_spirv);
        d.fragment[..s.fragment_spirv.len()].copy_from_slice(&s.fragment_spirv);
        for (v, input) in d.vertices.chunks_exact_mut(4).zip(&s.vertices) {
            v.copy_from_slice(input);
        }
        out.functions[n] = z.function;
        out.writes[n] = u32::from(z.write);
        if n == 0 {
            out.initial.copy_from_slice(&z.initial);
        }
    }
    out.expected = depth_expected(i);
    out.pixels = verify(i, &m, w)?;
    out.status = 0;
    out.draw_count = 2;
    out.image_bytes = 512;
    Ok(())
}
fn reject(page: Vec<u8>, i: usize) -> Result<(), String> {
    let (m, g) = textures::setup(page.clone())?;
    let err = ChannelProcessor::default()
        .prepare(&g, &textures::wire(0, words(i)))
        .err()
        .ok_or("Invalid projection accepted")?;
    check(err.contains("UnsupportedDrawState"))?;
    let mut after = vec![0; page.len()];
    m.read(0x1000, &mut after).map_err(|e| e.to_string())?;
    check(after == page)
}
fn mutate_position(page: &mut [u8], at: usize, p: [f32; 4]) {
    for (n, v) in p.into_iter().enumerate() {
        textures::word(page, at + n * 4, v.to_bits());
    }
}
fn core(i: usize) -> Result<usize, String> {
    match i {
        0 => {
            for n in 0..16 {
                let (m, w) = plan(n)?;
                verify(n, &m, w)?;
            }
            Ok(16)
        }
        1 => {
            for w in [
                0.25,
                0.5,
                1.,
                1.5,
                2.,
                3.,
                4.,
                16.,
                f32::MIN_POSITIVE,
                2f32.powi(125),
            ] {
                let mut p = PAGES[2].to_vec();
                for (n, c) in [0, 1, 2, 2, 1, 3].into_iter().enumerate() {
                    let (x, y) = [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)][c];
                    mutate_position(&mut p, 0xd00 + n * 32, [x * w, y * w, 0., w]);
                }
                let (m, g) = textures::setup(p)?;
                let (_, job) = ChannelProcessor::default().prepare(&g, &textures::wire(0, 142))?;
                job.commit();
                check(textures::pixels(&m)? == expected(2))?;
            }
            Ok(10)
        }
        2 => {
            for w in [
                0.,
                -0.,
                -1.,
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::from_bits(1),
                f32::MAX,
            ] {
                let mut p = PAGES[2].to_vec();
                textures::word(&mut p, 0xd0c, w.to_bits());
                reject(p, 2)?;
            }
            Ok(8)
        }
        3 => {
            for p0 in [
                [-0.75, -0.5, 0., 0.5],
                [0.75, -0.5, 0., 0.5],
                [-0.5, -0.75, 0., 0.5],
                [-0.5, 0.75, 0., 0.5],
                [-0.5, -0.5, -0.25, 0.5],
                [-0.5, -0.5, 0.75, 0.5],
            ] {
                let mut p = PAGES[2].to_vec();
                mutate_position(&mut p, 0xd00, p0);
                reject(p, 2)?;
            }
            Ok(6)
        }
        4 => {
            for w in [0.5, 1., 1.5, 2., 3., 4.] {
                let mut p = PAGES[2].to_vec();
                mutate_position(&mut p, 0xd00, [-0.999 * w, -w, 0., w]);
                reject(p, 2)?;
            }
            Ok(6)
        }
        5 => {
            for n in [2, 6, 8, 9] {
                let (m, w) = plan(n)?;
                check(w.draws()[0].vertices.iter().any(|v| v[3] != 1f32.to_bits()))?;
                verify(n, &m, w)?;
            }
            Ok(4)
        }
        6 => {
            for at in [0x1300, 0x1310, 0x1320, 0x1330, 0x1340, 0x1350] {
                let mut p = PAGES[12].to_vec();
                textures::word(&mut p, at + 12, 0);
                reject(p, 12)?;
            }
            Ok(6)
        }
        7 => {
            let (mut m, mut g) = textures::setup(PAGES[2].to_vec())?;
            let old = m
                .pin(0x1000, 4096, Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            let (_, w) = ChannelProcessor::default().prepare(&g, &textures::wire(0, 142))?;
            m.write(0x1d00, &[0; 192]).map_err(|e| e.to_string())?;
            let id = g
                .pin(GpuVa(1 << 32), 1, Access::Read)
                .map_err(|e| e.to_string())?
                .mapping_ids()
                .next()
                .ok_or("No map")?;
            g.unmap(id).map_err(|e| e.to_string())?;
            m.unmap(0x1000, 4096).map_err(|e| e.to_string())?;
            m.map(0x1000, vec![0xcc; 4096], Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            let replacement = g
                .map(
                    GpuVa(1 << 32),
                    m.pin(0x1000, 4096, Permissions::READ_WRITE)
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            w.commit();
            let mut actual = [0; 512];
            old.read(0x800, &mut actual).map_err(|e| e.to_string())?;
            check(actual == expected(2) && textures::pixels(&m)? == [0xcc; 512])?;
            g.unmap(replacement).map_err(|e| e.to_string())?;
            m.unmap(0x1000, 4096).map_err(|e| e.to_string())?;
            drop(old);
            check(m.resident_bytes() == 0)?;
            Ok(1)
        }
        8 => {
            let (m, g) = textures::setup(PAGES[2].to_vec())?;
            let prepare = |limits| {
                let mut q =
                    SubmissionQueue::new(&g, QueueLimits::default()).map_err(|e| e.to_string())?;
                let id = q
                    .submit(&g, &textures::wire(0, 142))
                    .map_err(|e| e.to_string())?;
                q.prepare_front(&g, id, limits).map_err(|e| e.to_string())
            };
            check(
                prepare(ExecutionLimits {
                    bytes: 1,
                    launches: 64,
                })
                .is_err(),
            )?;
            drop(prepare(ExecutionLimits::default())?);
            check(textures::pixels(&m)? == [0; 512])?;
            prepare(ExecutionLimits::default())?.commit();
            check(textures::pixels(&m)? == expected(2))?;
            Ok(3)
        }
        9 => {
            for (n, target) in [(2, 6), (8, 11), (12, 13)] {
                let (mut m, g) = textures::setup(PAGES[n].to_vec())?;
                let (next, w) =
                    ChannelProcessor::default().prepare(&g, &textures::wire(0, words(n)))?;
                w.commit();
                check(textures::pixels(&m)? == expected(n))?;
                // Replace resident commands and vertices with a different valid
                // projection. Retained channel must capture the new positions.
                m.write(0x1000, PAGES[target]).map_err(|e| e.to_string())?;
                let (_, w) = next.prepare(&g, &textures::wire(0, words(target)))?;
                check(w.draws()[0].vertices != plan(n)?.1.draws()[0].vertices)?;
                verify(target, &m, w)?;
            }
            Ok(3)
        }
        10 => {
            for n in [2, 8, 12] {
                let (mut m, mut g) = textures::setup(PAGES[n].to_vec())?;
                let (_, w) =
                    ChannelProcessor::default().prepare(&g, &textures::wire(0, words(n)))?;
                drop(w);
                let id = g
                    .pin(GpuVa(1 << 32), 1, Access::Read)
                    .map_err(|e| e.to_string())?
                    .mapping_ids()
                    .next()
                    .unwrap();
                g.unmap(id).map_err(|e| e.to_string())?;
                m.unmap(0x1000, PAGES[n].len()).map_err(|e| e.to_string())?;
                check(m.resident_bytes() == 0)?;
            }
            Ok(3)
        }
        11 => {
            for cycle in 0..64 {
                let n = cycle % 16;
                let (mut m, mut g) = textures::setup(PAGES[n].to_vec())?;
                let (_, w) =
                    ChannelProcessor::default().prepare(&g, &textures::wire(0, words(n)))?;
                verify(n, &m, w)?;
                let id = g
                    .pin(GpuVa(1 << 32), 1, Access::Read)
                    .map_err(|e| e.to_string())?
                    .mapping_ids()
                    .next()
                    .unwrap();
                g.unmap(id).map_err(|e| e.to_string())?;
                m.unmap(0x1000, PAGES[n].len()).map_err(|e| e.to_string())?;
                check(m.resident_bytes() == 0)?;
            }
            Ok(64)
        }
        12 => {
            let mut count = 0;
            for vertex in [0, 2, 5] {
                for component in 0..3 {
                    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                        let mut page = PAGES[2].to_vec();
                        textures::word(
                            &mut page,
                            0xd00 + vertex * 32 + component * 4,
                            value.to_bits(),
                        );
                        reject(page, 2)?;
                        count += 1;
                    }
                }
            }
            Ok(count)
        }
        13 => {
            for p0 in [
                [-0.5, -0.5, 0., 0.5],
                [0.5, -0.5, 0., 0.5],
                [-0.5, 0.5, 0., 0.5],
                [-0.5, -0.5, 0.5, 0.5],
            ] {
                let mut page = PAGES[2].to_vec();
                mutate_position(&mut page, 0xd00, p0);
                let (m, g) = textures::setup(page)?;
                let (_, w) = ChannelProcessor::default().prepare(&g, &textures::wire(0, 142))?;
                check(textures::pixels(&m)? == [0; 512])?;
                w.commit();
            }
            Ok(4)
        }
        14 => {
            for (n, page) in PAGES.iter().enumerate().skip(12) {
                let (mut m, g) = textures::setup(page.to_vec())?;
                let (_, w) = ChannelProcessor::default().prepare(&g, &textures::wire(0, 162))?;
                m.write(0x2000, &[0; 512]).map_err(|e| e.to_string())?;
                m.write(0x1d00, &[0; 96]).map_err(|e| e.to_string())?;
                m.write(0x2300, &[0; 96]).map_err(|e| e.to_string())?;
                w.commit();
                check(textures::pixels(&m)? == expected(n))?;
                let mut depth = [0; 512];
                m.read(0x2000, &mut depth).map_err(|e| e.to_string())?;
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
                for (y, row) in sectors.iter().enumerate() {
                    for x in 0..16 {
                        let at = row[x / 4] * 16 + (x % 4) * 4;
                        // A read-only draw captures comparison input but does not
                        // overwrite a later external memory change on commit.
                        let want = if n == 14 {
                            0
                        } else {
                            depth_expected(n)[y * 16 + x]
                        };
                        check(depth[at..at + 4] == want.to_le_bytes())?;
                    }
                }
            }
            Ok(4)
        }
        15 => {
            for empty in [false, true] {
                let mut page = PAGES[2].to_vec();
                if empty {
                    for at in (0..568).step_by(8) {
                        if u32::from_le_bytes(page[at..at + 4].try_into().unwrap()) & 8191
                            == 0xe04 / 4
                        {
                            textures::word(&mut page, at + 4, (16 << 16) | 16);
                        }
                    }
                } else {
                    for n in 1..6 {
                        let v = page[0xd00..0xd20].to_vec();
                        page[0xd00 + n * 32..0xd20 + n * 32].copy_from_slice(&v);
                    }
                }
                textures::word(&mut page, 0xd0c, 0);
                reject(page, 2)?;
            }
            Ok(2)
        }
        _ => Err("Unknown projection core case".into()),
    }
}
pub(crate) fn cases() -> Vec<TextureCase> {
    [
        "projection-sixteen-independent-render-oracles",
        "projection-normal-and-nonpower-w",
        "projection-invalid-w-atomic-rejection",
        "projection-six-clip-planes-rejected",
        "projection-off-grid-rejected",
        "projection-screen-linear-attributes-with-variable-w",
        "projection-late-invalid-depth-position-rollback",
        "projection-owned-inputs-unmap-and-reuse",
        "projection-budget-drop-and-retry",
        "projection-retained-channel-recaptures-inputs",
        "projection-cancel-final-owner-release",
        "projection-sixty-four-resource-cycles",
        "projection-nonfinite-referenced-position-rejected",
        "projection-exact-clip-boundaries",
        "projection-depth-input-snapshot-and-read-only-preservation",
        "projection-invalid-position-before-empty-or-degenerate-discard",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        let r =
            std::panic::catch_unwind(|| core(i)).unwrap_or_else(|_| Err("Projection panic".into()));
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
    macro_rules! case {
        ($name:ident,$i:expr) => {
            #[test]
            fn $name() {
                assert!(super::core($i).unwrap() > 0);
            }
        };
    }
    case!(sixteen_render_oracles, 0);
    case!(normal_and_nonpower_w, 1);
    case!(invalid_w, 2);
    case!(clip_planes, 3);
    case!(off_grid, 4);
    case!(linear_varyings, 5);
    case!(late_rollback, 6);
    case!(input_lifetime, 7);
    case!(budget_drop_retry, 8);
    case!(retained_channel, 9);
    case!(cancel_release, 10);
    case!(resource_cycles, 11);
    case!(nonfinite_positions, 12);
    case!(clip_boundaries, 13);
    case!(depth_snapshot, 14);
    case!(discard_cannot_hide_invalid_position, 15);
    #[test]
    fn extended_replay_ranges_preserve_abi_and_fail_closed() {
        for i in 3..11 {
            let r = crate::textures::replay(i);
            assert_eq!(r.status, 0, "{}", String::from_utf8_lossy(&r.error));
            assert_eq!(r.pixels, super::expected(i as usize - 3));
        }
        for i in 3..7 {
            let r = crate::indexed::replay(i);
            assert_eq!(r.status, 0, "{}", String::from_utf8_lossy(&r.error));
            assert_eq!(r.pixels, super::expected(i as usize + 5));
        }
        for i in 11..15 {
            let r = crate::depth::replay(i);
            assert_eq!(r.status, 0, "{}", String::from_utf8_lossy(&r.error));
            assert_eq!(r.pixels, super::expected(i as usize + 1));
        }
        let t = crate::textures::replay(11);
        assert!(t.status != 0 && t.draw_count == 0 && t.image_bytes == 0);
        let t = crate::indexed::replay(7);
        assert!(t.status != 0 && t.draw_count == 0 && t.image_bytes == 0);
        let t = crate::depth::replay(15);
        assert!(t.status != 0 && t.draw_count == 0 && t.image_bytes == 0);
    }
}
