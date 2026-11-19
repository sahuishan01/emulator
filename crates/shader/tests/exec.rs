use emulator_shader::cfg::build;
use emulator_shader::exec::{compare, run, Environment, ExecError, State};
use emulator_shader::maxwell::{list, Instr};

const EXIT: u64 = 0xe30000000007000f;
const PT_GUARD: u64 = 7 << 16;

struct Env;
impl Environment for Env {
    fn constant(&self, bank: u8, offset: u32) -> Option<u32> {
        (bank == 1 && offset == 4).then_some(0.5f32.to_bits())
    }
    fn attribute(&self, attr: u16) -> Option<u32> {
        (attr == 0x80).then_some(0.25f32.to_bits())
    }
}

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
fn execute(words: &[u64]) -> Result<State, ExecError> {
    let l = list(&code(words));
    let g = build(&l).expect("cfg");
    run(&l, &g, &Env, State::default(), 1000)
}

#[test]
fn float_alu_predicates_and_branch() {
    let fadd_imm = (0x3858u64 << 48) | (0x3f800 << 20) | PT_GUARD | 1;
    let fsetp_gt = (0x36b4u64 << 48) | (7 << 39) | (0x40200 << 20) | PT_GUARD | (1 << 8) | 7;
    let bra_p0 = 0xe240_0000_0000_000fu64 | 8 << 20;
    let ipa = (0xe000u64 << 48) | (0x80 << 28) | (0xff << 20) | PT_GUARD | (0xff << 8) | 4;
    // FFMA R5, R1, c[1][4], R1  -> 3 * 0.5 + 3
    let ffma_cbuf = (0x4980u64 << 48) | (1 << 39) | (1 << 34) | (1 << 20) | PT_GUARD | (1 << 8) | 5;
    let s = execute(&[
        mov32i(0, 2.0f32.to_bits(), 7),
        fadd_imm,
        fsetp_gt,
        bra_p0,
        mov32i(2, 5, 7),
        mov32i(3, 7, 0),
        ipa,
        ffma_cbuf,
        EXIT,
    ])
    .expect("runs");
    assert_eq!(f32::from_bits(s.registers[1]), 3.0);
    assert!(s.predicates[0]);
    assert_eq!(s.registers[2], 0, "skipped by the branch");
    assert_eq!(s.registers[3], 7);
    assert_eq!(f32::from_bits(s.registers[4]), 0.25);
    assert_eq!(f32::from_bits(s.registers[5]), 4.5);
}

#[test]
fn explicit_failures() {
    // XMAD is decoded but has no semantics yet.
    let xmad = (0x5b00u64 << 48) | PT_GUARD;
    assert!(matches!(
        execute(&[xmad, EXIT]),
        Err(ExecError::Unsupported {
            at: 8,
            instr: Instr::Xmad { .. }
        })
    ));
    // Unbound constant buffer.
    let mov_cbuf = (0x4c98u64 << 48) | (3 << 34) | PT_GUARD;
    assert_eq!(
        execute(&[mov_cbuf, EXIT]),
        Err(ExecError::Constant {
            at: 8,
            bank: 3,
            offset: 0
        })
    );
    // Infinite loop hits the step budget.
    let self_loop = 0xe2400fffff07000fu64;
    assert_eq!(execute(&[self_loop]), Err(ExecError::StepBudget));
}

#[test]
fn compare_codes() {
    let nan = f32::NAN;
    assert_eq!(compare(1, 1.0, 2.0), Some(true));
    assert_eq!(compare(1, nan, 2.0), Some(false));
    assert_eq!(compare(9, nan, 2.0), Some(true));
    assert_eq!(compare(5, nan, 2.0), Some(false));
    assert_eq!(compare(13, nan, 2.0), Some(true));
    assert_eq!(compare(8, nan, 0.0), Some(true));
    assert_eq!(compare(16, 0.0, 0.0), None);
}
