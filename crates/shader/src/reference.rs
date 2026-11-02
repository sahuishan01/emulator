//! Owned fixtures and portable decoder/IR/emitter regression batch.
use crate::{decode, emit, Error, Program, Stage};
pub const FIXTURES: [(&str, Stage, &[u8]); 4] = [
    (
        "vertex-passthrough",
        Stage::Vertex,
        include_bytes!("../../../examples/shaders/passthrough.vert.bin"),
    ),
    (
        "vertex-transform",
        Stage::Vertex,
        include_bytes!("../../../examples/shaders/transform.vert.bin"),
    ),
    (
        "fragment-green",
        Stage::Fragment,
        include_bytes!("../../../examples/shaders/green.frag.bin"),
    ),
    (
        "fragment-red",
        Stage::Fragment,
        include_bytes!("../../../examples/shaders/red.frag.bin"),
    ),
];
const NOP: u64 = 0x50b0000000070f00;
const EXIT: u64 = 0xe30000000007000f;
const MOV: u64 = 0x010000000007f000;
#[derive(Default, Debug)]
pub struct Counts {
    pub instructions: usize,
    pub spirv_words: usize,
    pub invocations: usize,
    pub rejected: usize,
}
#[derive(Debug)]
pub struct Case {
    pub name: &'static str,
    pub passed: bool,
    pub counts: Counts,
    pub error: String,
}
fn code(instructions: &[u64]) -> Vec<u8> {
    let mut words = instructions.to_vec();
    while words.len() % 3 != 0 {
        words.push(NOP);
    }
    let mut out = Vec::new();
    for chunk in words.chunks(3) {
        out.extend_from_slice(&0x001f8000fc0007e0u64.to_le_bytes());
        for w in chunk {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }
    out
}
fn constant(r: u8, bits: u32) -> u64 {
    MOV | u64::from(r) | (u64::from(bits) << 20)
}
fn patch(bytes: &[u8], offset: usize, w: u64) -> Vec<u8> {
    let mut result = bytes.to_vec();
    result[offset..offset + 8].copy_from_slice(&w.to_le_bytes());
    result
}
fn word(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("word"))
}
impl Counts {
    fn translate(&mut self, bytes: &[u8], stage: Stage) -> Result<Program, String> {
        let p = decode(bytes, stage).map_err(|e| e.to_string())?;
        let spv = emit(&p).map_err(|e| e.to_string())?;
        if spv.first() != Some(&0x07230203) {
            return Err("SPIR-V magic mismatch".into());
        }
        self.instructions += p.instruction_count();
        self.spirv_words += spv.len();
        Ok(p)
    }
    fn evaluate(&mut self, p: &Program, input: [u32; 4], expected: [u32; 4]) -> Result<(), String> {
        if p.evaluate(input) != expected {
            return Err("Independent invocation oracle mismatch".into());
        }
        self.invocations += 1;
        Ok(())
    }
    fn reject(
        &mut self,
        bytes: &[u8],
        stage: Stage,
        kind: fn(&Error) -> bool,
    ) -> Result<(), String> {
        match decode(bytes, stage) {
            Err(e) if kind(&e) => {
                self.rejected += 1;
                Ok(())
            }
            Err(e) => Err(format!("Unexpected rejection: {e}")),
            Ok(_) => Err("Unsupported shader accepted".into()),
        }
    }
}
fn fixture(index: usize, c: &mut Counts) -> Result<(), String> {
    let (_, stage, bytes) = FIXTURES[index];
    let p = c.translate(bytes, stage)?;
    for input in [
        [-1., -1., 0., 1.],
        [3., -1., 0., 1.],
        [-1., 3., 0., 1.],
        [0.25, -0.5, 0.75, 1.],
    ] {
        let expected = match index {
            0 => input,
            1 => [input[0] * 0.5, input[1] * 0.5, input[2], input[3]],
            2 => [0., 1., 0., 1.],
            _ => [1., 0., 0., 1.],
        };
        c.evaluate(&p, input.map(f32::to_bits), expected.map(f32::to_bits))?;
    }
    Ok(())
}
fn run_case(index: usize, c: &mut Counts) -> Result<(), String> {
    if index < 4 {
        return fixture(index, c);
    }
    let vertex = FIXTURES[0].2;
    let fragment = FIXTURES[2].2;
    let unsupported = |e: &Error| matches!(e, Error::Unsupported { .. });
    match index {
        4 => {
            for bits in [
                0, 0x80000000, 1, 0x007fffff, 0x7f800000, 0xff800000, 0x7fc12345, 0xffffffff,
            ] {
                let bytes = code(&[
                    constant(0, bits),
                    constant(1, bits),
                    constant(2, bits),
                    constant(3, bits),
                    EXIT,
                ]);
                let p = c.translate(&bytes, Stage::Fragment)?;
                c.evaluate(&p, [0; 4], [bits; 4])?;
            }
        }
        5 => {
            let bytes = code(&[
                constant(0, 1f32.to_bits()),
                constant(255, 2f32.to_bits()),
                (0x5c98078000070000 | (255 << 20) | 1),
                constant(2, 1f32.to_bits()),
                constant(3, 1f32.to_bits()),
                EXIT,
            ]);
            // MOV R1,RZ must remain zero even after a discarded RZ write.
            let p = c.translate(&bytes, Stage::Fragment)?;
            c.evaluate(&p, [0; 4], [1., 0., 1., 1.].map(f32::to_bits))?;
        }
        6 => {
            let mut ops = vec![];
            for i in 0..4u64 {
                ops.push(0xefd87f800007ff00 | i | ((0x80 + i * 4) << 20));
            }
            for i in 0..4u64 {
                ops.push(0xeff07f800007ff00 | i | ((0x70 + i * 4) << 20));
            }
            ops.push(EXIT);
            let p = c.translate(&code(&ops), Stage::Vertex)?;
            c.evaluate(&p, [11, 22, 33, 44], [11, 22, 33, 44])?;
        }
        7 => {
            for predicate in 0..16u64 {
                if predicate != 7 {
                    c.reject(
                        &patch(
                            fragment,
                            8,
                            (word(fragment, 8) & !(15 << 16)) | (predicate << 16),
                        ),
                        Stage::Fragment,
                        unsupported,
                    )?;
                }
            }
            c.reject(
                &patch(vertex, 24, EXIT | (1 << 19)),
                Stage::Vertex,
                unsupported,
            )?;
        }
        8 => {
            for bit in [8, 12, 52, 53, 54, 55] {
                c.reject(
                    &patch(fragment, 8, word(fragment, 8) ^ (1 << bit)),
                    Stage::Fragment,
                    unsupported,
                )?;
            }
            for bit in [39, 40, 44, 45, 46, 47, 48, 49, 50] {
                c.reject(
                    &patch(fragment, 16, word(fragment, 16) ^ (1 << bit)),
                    Stage::Fragment,
                    unsupported,
                )?;
            }
        }
        9 => {
            c.reject(vertex, Stage::Fragment, |e| {
                matches!(e, Error::Interface { .. })
            })?;
            for attr in [0x70, 0x81, 0x84, 0x100, 0x3fc] {
                let w = (word(vertex, 8) & !(0x3ff << 20)) | (attr << 20);
                c.reject(&patch(vertex, 8, w), Stage::Vertex, |e| {
                    matches!(e, Error::Interface { .. })
                })?;
            }
            c.reject(
                &patch(vertex, 8, (word(vertex, 8) & !255) | 253),
                Stage::Vertex,
                |e| matches!(e, Error::Interface { .. }),
            )?;
            c.reject(
                &patch(vertex, 8, word(vertex, 8) & !(1 << 8)),
                Stage::Vertex,
                unsupported,
            )?;
        }
        10 => {
            c.reject(&code(&[0x5c98078000570000, EXIT]), Stage::Fragment, |e| {
                matches!(e, Error::UndefinedRegister { .. })
            })?;
            c.reject(&code(&[EXIT]), Stage::Fragment, |e| {
                matches!(e, Error::MissingOutput)
            })?;
            c.reject(&code(&[EXIT]), Stage::Vertex, |e| {
                matches!(e, Error::MissingOutput)
            })?;
            c.reject(&patch(vertex, 8, NOP), Stage::Vertex, |e| {
                matches!(e, Error::UndefinedRegister { .. })
            })?;
        }
        11 => {
            for len in 0..64 {
                if len % 32 != 0 || len == 0 {
                    c.reject(&vec![0; len], Stage::Vertex, |e| matches!(e, Error::Length))?;
                }
            }
            c.reject(
                &patch(vertex, 0, word(vertex, 0) | (1 << 63)),
                Stage::Vertex,
                |e| matches!(e, Error::Schedule { .. }),
            )?;
            c.reject(&patch(vertex, 24, NOP), Stage::Vertex, |e| {
                matches!(e, Error::MissingExit)
            })?;
            c.reject(
                &code(&[EXIT, constant(0, 1f32.to_bits())]),
                Stage::Vertex,
                |e| matches!(e, Error::AfterExit { .. }),
            )?;
        }
        12 => {
            for w in [
                0,
                0xffffffffffffffff,
                0xe240000000070000,
                0xe32000000007000f,
                0xdeadbeef00070000,
            ] {
                c.reject(&patch(vertex, 8, w), Stage::Vertex, unsupported)?;
            }
        }
        13 => {
            c.reject(&vec![0; crate::MAX_CODE_BYTES + 32], Stage::Vertex, |e| {
                matches!(e, Error::Budget)
            })?;
            let mut full = vec![constant(0, 0); 191];
            full.push(EXIT);
            // Fill the other output registers first, then overwrite R0 many times.
            full[0] = constant(1, 0);
            full[1] = constant(2, 0);
            full[2] = constant(3, 0);
            let p = c.translate(&code(&full), Stage::Fragment)?;
            c.evaluate(&p, [0; 4], [0; 4])?;
            // Worst-case arithmetic IR expands four values per instruction and
            // exceeds the output budget without exposing a partial module.
            let mut ops = vec![constant(0, 0); 191];
            ops[0] = constant(1, 0);
            ops[1] = constant(2, 0);
            ops[2] = constant(3, 0);
            for op in &mut ops[4..] {
                *op = 0x5c58000000070000;
            }
            ops.push(EXIT);
            let p = decode(&code(&ops), Stage::Fragment).map_err(|e| e.to_string())?;
            if emit(&p) != Err(Error::Budget) {
                return Err("SPIR-V expansion budget not enforced".into());
            }
            c.rejected += 1;
        }
        14 => {
            let mut source = vertex.to_vec();
            let p = c.translate(&source, Stage::Vertex)?;
            let before = emit(&p).map_err(|e| e.to_string())?;
            source.fill(0);
            c.reject(&source, Stage::Vertex, unsupported)?;
            let clone = p.clone();
            if emit(&clone).map_err(|e| e.to_string())? != before || p != clone {
                return Err("Owned immutable plan changed".into());
            }
            c.evaluate(&clone, [1, 2, 3, 4], [1, 2, 3, 4])?;
        }
        _ => return Err("Unknown case".into()),
    }
    Ok(())
}
/// Each case is isolated and panic-contained. Counts report actual operations.
pub fn run() -> Vec<Case> {
    let names = [
        "vertex-passthrough",
        "vertex-transform",
        "fragment-green",
        "fragment-red",
        "bit-pattern-moves",
        "overwrite-and-zero-register",
        "scalar-attribute-components",
        "predication-rejected",
        "modifiers-rejected",
        "invalid-stage-and-interface",
        "undefined-read-and-output",
        "malformed-bundles-and-exits",
        "unsupported-opcodes-and-flow",
        "translation-budgets",
        "owned-plan-determinism",
    ];
    names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let mut counts = Counts::default();
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_case(i, &mut counts)))
                    .unwrap_or_else(|_| Err("Shader reference panic".into()));
            Case {
                name,
                passed: result.is_ok(),
                counts,
                error: result.err().unwrap_or_default(),
            }
        })
        .collect()
}
