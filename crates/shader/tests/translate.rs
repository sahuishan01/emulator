//! Maxwell -> SPIR-V translation: every program the executor runs must
//! translate to a module `spirv-val` accepts for Vulkan 1.1.
use emulator_shader::cfg::build;
use emulator_shader::maxwell::list;
use emulator_shader::sph::Header;
use emulator_shader::translate::{translate_fragment, TranslateError};

const EXIT: u64 = 0xe30000000007000f;
const PT_GUARD: u64 = 7 << 16;

fn code(words: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in words.chunks(3) {
        out.extend_from_slice(&0u64.to_le_bytes());
        for i in 0..3 {
            out.extend_from_slice(&chunk.get(i).copied().unwrap_or(0).to_le_bytes());
        }
    }
    out
}
fn mov32i(d: u64, imm: u32, guard: u64) -> u64 {
    (0x010u64 << 52) | (imm as u64) << 20 | 0xf << 12 | guard << 16 | d
}
fn pixel(omap: u32, depth: bool) -> Header {
    let mut words = [0u32; 20];
    words[0] = 2 | (3 << 5);
    words[18] = omap;
    words[19] = if depth { 2 } else { 0 };
    Header { words }
}
fn translate(words: &[u64], sph: &Header) -> Result<Vec<u32>, TranslateError> {
    let l = list(&code(words));
    let g = build(&l).expect("cfg");
    translate_fragment(&l, &g, sph)
}

/// Run spirv-val when installed (the test still checks translation without it).
fn validate(module: &[u32], name: &str) {
    let dir = std::env::temp_dir().join(format!("emu-spv-{}-{name}", std::process::id()));
    let bytes: Vec<u8> = module.iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(&dir, bytes).expect("write");
    let out = std::process::Command::new("spirv-val")
        .args(["--target-env", "vulkan1.1"])
        .arg(&dir)
        .output();
    let _ = std::fs::remove_file(&dir);
    match out {
        Ok(o) => assert!(
            o.status.success(),
            "{name}: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => eprintln!("spirv-val unavailable ({e}); skipped validation of {name}"),
    }
}

fn texs(d0: u64, d1: u64, a: u64, b: u64, handle: u64, mask: u64, kind: u64) -> u64 {
    (0xd8u64 << 56)
        | kind << 53
        | mask << 50
        | handle << 36
        | d1 << 28
        | b << 20
        | PT_GUARD
        | a << 8
        | d0
}
fn xmad(d: u64, a: u64, b: u64, c: u64, flags: u64) -> u64 {
    (0x5b00u64 << 48) | flags | c << 39 | b << 20 | PT_GUARD | a << 8 | d
}

#[test]
fn alu_branches_attributes_and_constants_validate() {
    let fadd_imm = (0x3858u64 << 48) | (0x3f800 << 20) | PT_GUARD | 1;
    let fsetp_gt = (0x36b4u64 << 48) | (7 << 39) | (0x40200 << 20) | PT_GUARD | (1 << 8) | 7;
    let bra_p0 = 0xe240_0000_0000_000fu64 | 8 << 20;
    let ipa = (0xe000u64 << 48)
        | (0xffu64 << 39)
        | (0b0111u64 << 47)
        | (0x80 << 28)
        | (0xff << 20)
        | PT_GUARD
        | (0xff << 8)
        | 4;
    let ffma_cbuf = (0x4980u64 << 48) | (1 << 39) | (1 << 34) | (1 << 20) | PT_GUARD | (1 << 8) | 5;
    let module = translate(
        &[
            mov32i(0, 2.0f32.to_bits(), 7),
            fadd_imm,
            fsetp_gt,
            bra_p0,
            mov32i(2, 5, 7),
            mov32i(3, 7, 0), // guarded by P0
            ipa,
            ffma_cbuf,
            xmad(6, 0, 1, 2, 2 << 50),
            EXIT,
        ],
        &pixel(0xf, false),
    )
    .expect("translates");
    validate(&module, "alu");
}

#[test]
fn textures_mrt_and_depth_validate() {
    let za_tex = 0xc0f8008120770407u64; // TEX.LZ.AOFFI R7, R4, R7, handle 8
    let module = translate(
        &[
            mov32i(4, 1.0f32.to_bits(), 7),
            mov32i(5, 3.0f32.to_bits(), 7),
            texs(10, 20, 4, 5, 8, 4, 2),
            za_tex,
            EXIT,
        ],
        // rt0.xyzw, rt1.x, depth.
        &pixel(0x1f, true),
    )
    .expect("translates");
    validate(&module, "tex");
}

#[test]
fn rejections_match_the_executor() {
    // XMAD CSFU, non-pixel header, sample-mask output.
    let csfu = (0x5b00u64 << 48) | (3 << 50) | PT_GUARD;
    assert!(matches!(
        translate(&[csfu, EXIT], &pixel(0xf, false)),
        Err(TranslateError::Unsupported { .. })
    ));
    let mut vertex = pixel(0xf, false);
    vertex.words[0] = 1;
    assert_eq!(translate(&[EXIT], &vertex), Err(TranslateError::NotPixel));
    let mut mask = pixel(0xf, false);
    mask.words[19] = 1;
    assert_eq!(translate(&[EXIT], &mask), Err(TranslateError::Outputs));
}

// ---------------------------------------------------------------------------
// Differential check: the translated shader rendered on a Vulkan device must
// give the same rt0 bits as the reference executor. Runs only when
// EMU_FRAG_RUN names a built `tools/spirv-run/frag_run` (lavapipe works) and
// glslangValidator is installed (host test tooling for the fixed vertex shader).

use emulator_shader::exec::{run, Environment, State};

const BANK: u8 = 1;
const HANDLE: u16 = 8;
const VARYING: [f32; 4] = [0.25, -1.5, 3.0, 0.0];

fn cbuf() -> Vec<u8> {
    let mut data = vec![0u8; 65536];
    for i in 0..16384u32 {
        let v = (i as f32) * 0.5 - 7.0;
        data[i as usize * 4..i as usize * 4 + 4].copy_from_slice(&v.to_bits().to_le_bytes());
    }
    data
}
fn texel(x: i32, y: i32) -> [f32; 4] {
    [x as f32, y as f32 + 10.0, 0.5, 0.25]
}

struct HostEnv(Vec<u8>);
impl Environment for HostEnv {
    fn constant(&self, bank: u8, offset: u32) -> Option<u32> {
        let o = offset as usize;
        (bank == BANK && o + 4 <= self.0.len())
            .then(|| u32::from_le_bytes(self.0[o..o + 4].try_into().expect("4")))
    }
    fn attribute(&self, attr: u16) -> Option<u32> {
        // One pixel at (0.5, 0.5), w = 1, so linear attr/w = attr.
        Some(match attr {
            0x70 | 0x74 => 0.5f32.to_bits(),
            0x78 => 0.0f32.to_bits(),
            0x7c => 1.0f32.to_bits(),
            0x80..=0x8c => VARYING[(attr as usize - 0x80) / 4].to_bits(),
            _ => return None,
        })
    }
    fn texture_2d(&self, handle: u16, x: f32, y: f32, lod: f32, o: [i32; 2]) -> Option<[f32; 4]> {
        // Nearest, clamp to edge, 4x4.
        if handle != HANDLE || lod != 0.0 {
            return None;
        }
        let ix = ((x * 4.0).floor() as i32 + o[0]).clamp(0, 3);
        let iy = ((y * 4.0).floor() as i32 + o[1]).clamp(0, 3);
        Some(texel(ix, iy))
    }
}

fn differential(words: &[u64], name: &str) {
    let Ok(harness) = std::env::var("EMU_FRAG_RUN") else {
        eprintln!("EMU_FRAG_RUN unset; skipped device check of {name}");
        return;
    };
    let l = list(&code(words));
    let g = build(&l).expect("cfg");
    let sph = pixel(0xf, false);
    let module = translate_fragment(&l, &g, &sph).expect("translates");
    validate(&module, name);
    let env = HostEnv(cbuf());
    let state = run(&l, &g, &env, State::default(), 10_000).expect("exec");
    let expected: Vec<u32> = (0..4).map(|i| state.registers[i]).collect();

    let dir = std::env::temp_dir().join(format!("emu-diff-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let vert_src = format!(
        "#version 450\nlayout(location=0) out vec4 v;\nvoid main(){{\n\
         vec2 p = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n\
         gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);\n\
         v = vec4({:?}, {:?}, {:?}, {:?});\n}}\n",
        VARYING[0], VARYING[1], VARYING[2], VARYING[3]
    );
    std::fs::write(dir.join("v.vert"), vert_src).expect("vert");
    let ok = std::process::Command::new("glslangValidator")
        .args(["-V", "-o"])
        .arg(dir.join("v.spv"))
        .arg(dir.join("v.vert"))
        .output()
        .expect("glslangValidator");
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stdout)
    );
    let bytes: Vec<u8> = module.iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(dir.join("f.spv"), bytes).expect("frag");
    let mut input = Vec::new();
    input.extend_from_slice(&u32::from(BANK).to_le_bytes());
    input.extend_from_slice(&u32::from(HANDLE).to_le_bytes());
    input.extend_from_slice(&env.0);
    for y in 0..4 {
        for x in 0..4 {
            for c in texel(x, y) {
                input.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    std::fs::write(dir.join("in.bin"), input).expect("input");
    let out = std::process::Command::new(&harness)
        .arg(dir.join("v.spv"))
        .arg(dir.join("f.spv"))
        .arg(dir.join("in.bin"))
        .output()
        .expect("harness");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got: Vec<u32> = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(|w| u32::from_str_radix(w, 16).expect("hex"))
        .collect();
    assert_eq!(
        got,
        expected,
        "{name}: device vs exec (as f32: {:?} vs {:?})",
        got.iter().map(|b| f32::from_bits(*b)).collect::<Vec<_>>(),
        expected
            .iter()
            .map(|b| f32::from_bits(*b))
            .collect::<Vec<_>>()
    );
}

fn fadd_imm(d: u64, a: u64, imm20: u64) -> u64 {
    (0x3858u64 << 48) | (imm20 << 20) | PT_GUARD | a << 8 | d
}

#[test]
fn device_matches_executor_alu_flow() {
    let fsetp_gt = (0x36b4u64 << 48) | (7 << 39) | (0x40200 << 20) | PT_GUARD | (1 << 8) | 7;
    let bra_p0 = 0xe240_0000_0000_000fu64 | 8 << 20;
    let ipa = (0xe000u64 << 48)
        | (0xffu64 << 39)
        | (0b0111u64 << 47)
        | (0x80 << 28)
        | (0xff << 20)
        | PT_GUARD
        | (0xff << 8)
        | 4;
    // FFMA R5, R1, c[1][0x40], R1
    let ffma_cbuf =
        (0x4980u64 << 48) | (1 << 39) | (0x10 << 20) | (1 << 34) | PT_GUARD | (1 << 8) | 5;
    differential(
        &[
            mov32i(0, 2.0f32.to_bits(), 7),
            fadd_imm(1, 0, 0x3f800), // R1 = 3
            fsetp_gt,                // P0 = R1 > 2.5
            bra_p0,
            mov32i(2, 5, 7), // skipped
            mov32i(3, 7, 0), // P0 guard
            ipa,             // R4 = 0.25
            ffma_cbuf,       // R5 = 3 * c + 3
            fadd_imm(0, 5, 0),
            fadd_imm(1, 4, 0),
            xmad(2, 3, 3, 3, 0), // 7*7+7
            EXIT,
        ],
        "alu",
    );
}

#[test]
fn device_matches_executor_textures() {
    let x = 1.5f32 / 4.0;
    let y = 2.5f32 / 4.0;
    let za_tex = 0xc0f8008120770407u64; // TEX.LZ.AOFFI R7, R4, R7, handle 8, mask G
    let tex_to_r3 = (za_tex & !0xff) | 3;
    differential(
        &[
            mov32i(4, x.to_bits(), 7),
            mov32i(5, y.to_bits(), 7),
            mov32i(7, 0x1f, 7),        // offset x = -1, y = 1
            texs(0, 2, 4, 5, 8, 4, 2), // RGBA -> R0, R1, R2, R3
            tex_to_r3,                 // G of texel (0, 3) -> R3
            EXIT,
        ],
        "tex",
    );
}
