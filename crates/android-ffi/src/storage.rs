//! Original guest texture storage corpus and capability-independent RGBA replay.
use crate::graphics::ClearOp;
use crate::textures::{
    self, check, pixels, setup, wire, word, TextureCase, TextureReplay, TexturedDraw,
};
use emulator_gpu::{ChannelProcessor, ExecutionLimits, GpuVa, QueueLimits, SubmissionQueue};
use emulator_memory::{Access, Permissions};
use emulator_session::{RunOutcome, Session, LOAD_BASE};
pub(crate) const IMAGE: &[u8] = include_bytes!("../../../examples/homebrew/nv-storage.nro");
pub(crate) const MESSAGE: &str = "Guest storage passed: tiles, compressed textures and lifetime.\n";
const WORDS: u32 = 142;
const PAGES: [&[u8]; 6] = [
    include_bytes!("../../../examples/gpu/storage-tiled-rgba-page.bin"),
    include_bytes!("../../../examples/gpu/storage-multigob-rgba-page.bin"),
    include_bytes!("../../../examples/gpu/storage-pitch-bc1-page.bin"),
    include_bytes!("../../../examples/gpu/storage-tiled-bc1-alpha-page.bin"),
    include_bytes!("../../../examples/gpu/storage-pitch-bc3-page.bin"),
    include_bytes!("../../../examples/gpu/storage-tiled-bc3-alpha-page.bin"),
];
const RGBA: [&[u8]; 6] = [
    include_bytes!("../../../examples/gpu/storage-tiled-rgba-rgba"),
    include_bytes!("../../../examples/gpu/storage-multigob-rgba-rgba"),
    include_bytes!("../../../examples/gpu/storage-pitch-bc1-rgba"),
    include_bytes!("../../../examples/gpu/storage-tiled-bc1-alpha-rgba"),
    include_bytes!("../../../examples/gpu/storage-pitch-bc3-rgba"),
    include_bytes!("../../../examples/gpu/storage-tiled-bc3-alpha-rgba"),
];
const DIMS: [(i32, i32); 6] = [(4, 4), (17, 33), (7, 5), (7, 9), (5, 7), (13, 9)];
fn expected(index: usize, updated: bool) -> [u8; 512] {
    let (w, h) = DIMS[index];
    let mut result = [0; 512];
    for y in 0..8i32 {
        for x in 0..16i32 {
            let center = (4..12).contains(&x) && (2..6).contains(&y);
            let (sx, sy) = if center {
                (
                    ((6 * x - 29) * w).div_euclid(32),
                    ((6 * y - 13) * h).div_euclid(16),
                )
            } else {
                (
                    ((6 * x - 13) * w).div_euclid(64),
                    ((6 * y - 5) * h).div_euclid(32),
                )
            };
            let sx = sx.clamp(0, w - 1);
            let sy = sy.clamp(0, h - 1);
            let a = ((sy * w + sx) * 4) as usize;
            let dst = ((y * 16 + x) * 4) as usize;
            result[dst..dst + 4].copy_from_slice(&RGBA[index][a..a + 4]);
            if updated {
                result[dst + 3] = [0, 255, 51, 102, 153, 204, 0, 255][((3 * sx + sy) % 8) as usize];
            }
        }
    }
    result
}
pub(crate) fn verify_guest(s: &Session) -> Result<(), String> {
    let mut page = [0; 4096];
    s.memory()
        .read(LOAD_BASE + 0x5000, &mut page)
        .map_err(|e| e.to_string())?;
    check(
        page[0x800..0xa00] == expected(0, false)
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
        .unwrap_or_else(|_| Err("Storage replay panic".into()));
    if let Err(e) = result {
        for (a, b) in out.error.iter_mut().zip(e.bytes().take(255)) {
            *a = b;
        }
    }
    out
}
fn fill(index: u32, out: &mut TextureReplay) -> Result<(), String> {
    let mut page = PAGES
        .get(index as usize)
        .ok_or("Unknown storage case")?
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
    for (value, draw) in out.draws.iter_mut().zip(work.draws()) {
        check(
            draw.texture
                .as_ref()
                .ok_or("Missing storage texture")?
                .pixels
                == RGBA[index as usize],
        )?;
        *value = textures::attributes(draw)?;
    }
    let stats = work.commit().1;
    check(stats.draw_launches == 2 && stats.clear_launches == 1 && stats.drawn_bytes == 640)?;
    out.pixels = pixels(&mem)?;
    check(out.pixels == expected(index as usize, false))?;
    out.draw_count = 2;
    out.image_bytes = 512;
    out.status = 0;
    Ok(())
}
fn rejected(page: Vec<u8>, words: u32) -> Result<(), String> {
    let (mem, gpu) = setup(page)?;
    check(
        ChannelProcessor::default()
            .prepare(&gpu, &wire(0, words))
            .is_err()
            && pixels(&mem)? == [0; 512],
    )
}
fn core_case(i: usize) -> Result<usize, String> {
    match i {
        0 => {
            for n in 0..6 {
                let r = replay(n);
                if r.status != 0 {
                    return Err(String::from_utf8_lossy(&r.error).into());
                }
                check(r.pixels == expected(n as usize, false))?;
            }
            Ok(3072)
        }
        1 => {
            let mut count = 0;
            for case in [0usize, 2, 4, 5] {
                for (at, value) in [
                    (0x600, 0),
                    (0x604, 0x401),
                    (0x608, 1 | (4 << 21)),
                    (0x60c, 6 << 3),
                    (0x60c, 1 << 6),
                    (0x610, (1 << 23) | 64),
                    (0x614, (1 << 31) | (1 << 16) | 3),
                    (0x618, 1),
                    (0x61c, 1),
                    (0x700, 0),
                    (0x704, 0x52),
                ] {
                    let mut p = PAGES[case].to_vec();
                    word(&mut p, at, value);
                    rejected(p, WORDS)?;
                    count += 1;
                }
            }
            Ok(count)
        }
        2 => {
            for case in [0usize, 1, 4, 5] {
                let (mut mem, gpu) = setup(PAGES[case].to_vec())?;
                let (_, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, WORDS))?;
                let address =
                    u32::from_le_bytes(PAGES[case][0x604..0x608].try_into().unwrap()) as u64;
                mem.write(0x1000 + address, &[0; 32])
                    .map_err(|e| e.to_string())?;
                check(work.draws()[0].texture.as_ref().unwrap().pixels == RGBA[case])?;
                work.commit();
                check(pixels(&mem)? == expected(case, false))?;
            }
            Ok(2048)
        }
        3 => {
            let size = PAGES[1].len();
            let (mut mem, mut gpu) = setup(PAGES[1].to_vec())?;
            let original = mem
                .pin(0x1000, size, Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            let (_, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, WORDS))?;
            let id = gpu
                .pin(GpuVa(1 << 32), 4, Access::Read)
                .map_err(|e| e.to_string())?
                .mapping_ids()
                .next()
                .ok_or("No mapping")?;
            gpu.unmap(id).map_err(|e| e.to_string())?;
            mem.unmap(0x1000, size).map_err(|e| e.to_string())?;
            mem.map(0x1000, vec![0xcc; size], Permissions::READ_WRITE)
                .map_err(|e| e.to_string())?;
            let new = gpu
                .map(
                    GpuVa(1 << 32),
                    mem.pin(0x1000, size, Permissions::READ_WRITE)
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            work.commit();
            let mut old = [0; 512];
            original.read(0x800, &mut old).map_err(|e| e.to_string())?;
            check(old == expected(1, false) && pixels(&mem)? == [0xcc; 512])?;
            gpu.unmap(new).map_err(|e| e.to_string())?;
            mem.unmap(0x1000, size).map_err(|e| e.to_string())?;
            drop(original);
            check(mem.resident_bytes() == 0)?;
            Ok(512)
        }
        4 => {
            let (mut mem, gpu) = setup(PAGES[4].to_vec())?;
            let (next, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, WORDS))?;
            work.commit();
            for row in 0..2 {
                for col in 0..2 {
                    mem.write(0x1400 + row * 32 + col * 16, &[0, 255])
                        .map_err(|e| e.to_string())?;
                }
            }
            let (_, work) = next.prepare(&gpu, &wire(0x300, 2))?;
            check(work.commit().1.drawn_bytes == 128)?;
            let mut oracle = expected(4, false);
            let changed = expected(4, true);
            for y in 2..6 {
                for x in 4..12 {
                    let a = (y * 16 + x) * 4;
                    oracle[a..a + 4].copy_from_slice(&changed[a..a + 4]);
                }
            }
            check(pixels(&mem)? == oracle)?;
            Ok(512)
        }
        5 => {
            for alias in [false, true] {
                let mut p = PAGES[2].to_vec();
                word(&mut p, 0x604, 0x800);
                if alias {
                    word(&mut p, 0x608, (2 << 21) | 2);
                }
                let (mem, mut gpu) = setup(p)?;
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
        6 => {
            let mut p = PAGES[1].to_vec();
            p.truncate(8192);
            rejected(p, WORDS)?;
            let mut p = PAGES[5].to_vec();
            word(&mut p, 0x608, (3 << 21) | 2);
            let (mut mem, mut gpu) = setup(p)?;
            mem.map(0x5000, vec![0; 8192], Permissions::WRITE)
                .map_err(|e| e.to_string())?;
            gpu.map(
                GpuVa(2 << 32),
                mem.pin(0x5000, 8192, Permissions::WRITE)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            check(
                ChannelProcessor::default()
                    .prepare(&gpu, &wire(0, WORDS))
                    .is_err()
                    && pixels(&mem)? == [0; 512],
            )?;
            Ok(2)
        }
        7 => {
            rejected(PAGES[5].to_vec(), WORDS + 2)?;
            let mut p = PAGES[0].to_vec();
            let at = (0..WORDS as usize * 4)
                .step_by(8)
                .rfind(|a| {
                    u32::from_le_bytes(p[*a..*a + 4].try_into().unwrap()) & 0x1fff == 0x1214 / 4
                })
                .unwrap();
            word(&mut p, at + 4, 0x40060001);
            rejected(p, WORDS)?;
            Ok(1024)
        }
        8 => {
            let (mem, gpu) = setup(PAGES[1].to_vec())?;
            let (_, work) = ChannelProcessor::default().prepare(&gpu, &wire(0, WORDS))?;
            let spv = work
                .draws()
                .iter()
                .map(|d| (d.vertex_spirv.len() + d.fragment_spirv.len()) * 4)
                .sum::<usize>();
            drop(work);
            let without_decoded = 512 + 640 + 6688 + 6720 + spv;
            for (bytes, valid) in [(without_decoded, false), (without_decoded + 2 * 2244, true)] {
                let mut q = SubmissionQueue::new(&gpu, QueueLimits::default())
                    .map_err(|e| e.to_string())?;
                let id = q.submit(&gpu, &wire(0, WORDS)).map_err(|e| e.to_string())?;
                let result = q.prepare_front(
                    &gpu,
                    id,
                    ExecutionLimits {
                        bytes,
                        launches: 64,
                    },
                );
                check(result.is_ok() == valid)?;
            }
            check(pixels(&mem)? == [0; 512])?;
            Ok(2)
        }
        9 => {
            guest()?;
            Ok(512)
        }
        _ => Err("Unknown storage core case".into()),
    }
}
pub(crate) fn cases() -> Vec<TextureCase> {
    [
        "storage-six-independent-layout-and-format-images",
        "storage-descriptor-modes-and-bounds-rejected",
        "storage-immutable-decoded-input-snapshots",
        "storage-padded-capture-unmap-and-address-reuse",
        "storage-retained-state-recaptures-bc3-alpha",
        "storage-compressed-direct-and-alias-feedback",
        "storage-truncated-and-unreadable-input",
        "storage-late-and-second-draw-rollback",
        "storage-decoded-snapshot-budget-and-cancel",
        "storage-guest-nro-rollback-and-lifetime",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        let r = std::panic::catch_unwind(|| core_case(i))
            .unwrap_or_else(|_| Err("Storage core panic".into()));
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
    fn guest_storage_bridge_and_portable_cases() {
        for case in super::cases() {
            assert!(case.passed, "{}: {}", case.name, case.error);
        }
        let bad = super::replay(6);
        assert_eq!((bad.status, bad.draw_count, bad.image_bytes), (1, 0, 0));
    }
}
