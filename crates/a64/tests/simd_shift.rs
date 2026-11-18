//! AdvSIMD three-same register shifts (SQSHL/UQSHL, SRSHL/URSHL,
//! SQRSHL/UQRSHL) and SQDMULH/SQRDMULH, with hand-computed known answers.

use emulator_a64::Cpu;
use emulator_memory::{AddressSpace, Permissions};

const CODE: u64 = 0x10_0000;

struct Guest {
    cpu: Cpu,
    memory: AddressSpace,
}

impl Guest {
    fn new() -> Self {
        Self {
            cpu: Cpu::new(CODE, 0x8000_0000),
            memory: AddressSpace::default(),
        }
    }

    /// Runs one instruction word at a fresh address.
    fn exec(&mut self, word: u32) {
        let page = CODE + self.memory.mapped_bytes() as u64;
        let mut bytes = vec![0u8; 4096];
        bytes[..4].copy_from_slice(&word.to_le_bytes());
        self.memory
            .map(page, bytes, Permissions::READ_EXECUTE)
            .expect("code page");
        self.cpu.pc = page;
        self.cpu
            .step(&mut self.memory)
            .unwrap_or_else(|error| panic!("{word:#010x}: {error:?}"));
    }

    /// `op Vd, Vn, Vm` with 32-bit lanes: returns Vd's lanes.
    fn s4(&mut self, u: bool, opcode: u32, a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
        self.cpu.v[1] = lanes32(a);
        self.cpu.v[2] = lanes32(b);
        self.exec(vector(true, u, 2, opcode, 0, 1, 2));
        let d = self.cpu.v[0];
        [0, 1, 2, 3].map(|i| (d >> (32 * i)) as u32)
    }
}

fn lanes32(x: [u32; 4]) -> u128 {
    x.iter()
        .enumerate()
        .fold(0, |acc, (i, &v)| acc | (u128::from(v) << (32 * i)))
}

/// AdvSIMD three-same, vector form.
fn vector(q: bool, u: bool, size: u32, opcode: u32, d: u32, n: u32, m: u32) -> u32 {
    u32::from(q) << 30
        | u32::from(u) << 29
        | 0b01110 << 24
        | size << 22
        | 1 << 21
        | m << 16
        | opcode << 11
        | 1 << 10
        | n << 5
        | d
}

/// AdvSIMD scalar three-same.
fn scalar(u: bool, size: u32, opcode: u32, d: u32, n: u32, m: u32) -> u32 {
    1 << 30 | u32::from(u) << 29 | 0b11110 << 24 | size << 22 | 1 << 21 | m << 16 | opcode << 11 | 1 << 10 | n << 5 | d
}

const SHL_SAT: u32 = 9;
const RSHL: u32 = 10;
const RSHL_SAT: u32 = 11;
const DMULH: u32 = 22;

#[test]
fn z_a_sqshl_word_decodes_as_vector_sqshl_4s() {
    assert_eq!(vector(true, false, 2, SHL_SAT, 5, 5, 4), 0x4ea4_4ca5);
}

#[test]
fn saturating_shifts_by_register() {
    let mut g = Guest::new();
    let neg = |x: i32| x as u32;
    // SQSHL: left shifts saturate, negative shifts are arithmetic.
    assert_eq!(
        g.s4(false, SHL_SAT, [1, 0x4000_0000, neg(-1), 0x7fff_ffff], [4, 1, 31, neg(-1)]),
        [16, 0x7fff_ffff, 0x8000_0000, 0x3fff_ffff]
    );
    assert_eq!(
        g.s4(false, SHL_SAT, [neg(-3), 0, neg(-8), 5], [40, 127, neg(-2), 0]),
        [0x8000_0000, 0, neg(-2), 5]
    );
    // UQSHL: unsigned saturation and logical right shifts.
    assert_eq!(
        g.s4(true, SHL_SAT, [1, 0x8000_0000, 0xffff_ffff, 8], [31, 1, neg(-32), neg(-3)]),
        [0x8000_0000, 0xffff_ffff, 0, 1]
    );
}

#[test]
fn rounding_shifts_by_register() {
    let mut g = Guest::new();
    let neg = |x: i32| x as u32;
    // SRSHL: right shifts round half up; left shifts wrap; >= width is 0.
    assert_eq!(
        g.s4(false, RSHL, [5, neg(-5), 7, 1], [neg(-1), neg(-1), neg(-2), 33]),
        [3, neg(-2), 2, 0]
    );
    assert_eq!(
        g.s4(false, RSHL, [0x4000_0001, neg(-1), 1, 3], [1, neg(-40), neg(-1), 30]),
        [0x8000_0002, 0, 1, 0xc000_0000]
    );
    // URSHL: the rounding carry is kept (no wrap to 0).
    assert_eq!(
        g.s4(true, RSHL, [0xffff_ffff, 3, 2, 1], [neg(-1), neg(-1), neg(-2), 0]),
        [0x8000_0000, 2, 1, 1]
    );
    // SQRSHL / UQRSHL: rounding and saturation together.
    assert_eq!(
        g.s4(false, RSHL_SAT, [0x7fff_ffff, neg(-7), 0x4000_0000, 0x7fff_ffff], [neg(-1), neg(-1), 1, 0]),
        [0x4000_0000, neg(-3), 0x7fff_ffff, 0x7fff_ffff]
    );
    assert_eq!(
        g.s4(true, RSHL_SAT, [0xffff_ffff, 0x8000_0001, 6, 0], [neg(-1), 1, neg(-2), 31]),
        [0x8000_0000, 0xffff_ffff, 2, 0]
    );
}

#[test]
fn byte_lanes_64_bit_vectors_and_scalar_forms() {
    let mut g = Guest::new();
    // SQSHL .8b: shift byte -128 is a full arithmetic right shift; the
    // upper half of the destination is cleared.
    g.cpu.v[0] = u128::MAX;
    g.cpu.v[1] = u128::from_le_bytes([0x9c, 0x64, 0x01, 0x80, 0x7f, 0x02, 0xff, 0x10, 1, 1, 1, 1, 1, 1, 1, 1]);
    g.cpu.v[2] = u128::from_le_bytes([0x80, 0x80, 0x07, 0x01, 0x7f, 0x06, 0x07, 0xfc, 0, 0, 0, 0, 0, 0, 0, 0]);
    g.exec(vector(false, false, 0, SHL_SAT, 0, 1, 2));
    assert_eq!(
        g.cpu.v[0],
        u128::from_le_bytes([0xff, 0x00, 0x7f, 0x80, 0x7f, 0x7f, 0x80, 0x01, 0, 0, 0, 0, 0, 0, 0, 0])
    );
    // Scalar SQSHL d0, d1, d2 saturates 2^62 << 1; upper 64 bits cleared.
    g.cpu.v[0] = u128::MAX;
    g.cpu.v[1] = 0x4000_0000_0000_0000 | (7 << 64);
    g.cpu.v[2] = 1;
    g.exec(scalar(false, 3, SHL_SAT, 0, 1, 2));
    assert_eq!(g.cpu.v[0], 0x7fff_ffff_ffff_ffff);
    // Scalar URSHL d: (2^64 - 1) rounded right by 1 carries into bit 63.
    g.cpu.v[1] = u128::from(u64::MAX);
    g.cpu.v[2] = 0xff;
    g.exec(scalar(true, 3, RSHL, 0, 1, 2));
    assert_eq!(g.cpu.v[0], 0x8000_0000_0000_0000);
    // Scalar SQRSHL s (32-bit) is allowed; scalar SRSHL needs 64-bit lanes.
    g.cpu.v[1] = 0x7fff_ffff;
    g.cpu.v[2] = 2;
    g.exec(scalar(false, 2, RSHL_SAT, 0, 1, 2));
    assert_eq!(g.cpu.v[0], 0x7fff_ffff);
    g.cpu.pc = CODE;
    let word = scalar(false, 2, RSHL, 0, 1, 2);
    let page = CODE + g.memory.mapped_bytes() as u64;
    let mut bytes = vec![0u8; 4096];
    bytes[..4].copy_from_slice(&word.to_le_bytes());
    g.memory.map(page, bytes, Permissions::READ_EXECUTE).unwrap();
    g.cpu.pc = page;
    assert!(g.cpu.step(&mut g.memory).is_err(), "scalar SRSHL s is unallocated");
}

#[test]
fn doubling_multiply_high_half() {
    let mut g = Guest::new();
    let neg = |x: i32| x as u32;
    // SQDMULH .4s: (2ab) >> 32, saturating only for MIN * MIN.
    assert_eq!(
        g.s4(false, DMULH, [0x4000_0000, 0x8000_0000, 3, neg(-0x4000_0000)], [0x4000_0000, 0x8000_0000, 5, 0x4000_0000]),
        [0x2000_0000, 0x7fff_ffff, 0, 0xe000_0000]
    );
    // SQRDMULH .8h rounds: 2 * 16384 * 1 + 2^15 = 2^16 -> 1.
    g.cpu.v[1] = 0x4000;
    g.cpu.v[2] = 0x0001;
    g.exec(vector(true, true, 1, DMULH, 0, 1, 2));
    assert_eq!(g.cpu.v[0], 1);
    // SQDMULH truncates the same product to 0.
    g.exec(vector(true, false, 1, DMULH, 0, 1, 2));
    assert_eq!(g.cpu.v[0], 0);
}

#[test]
fn neighbouring_z_a_simd_words_execute() {
    // The instructions around Z-A's SQSHL at 0x30bc5cc (DUP, BIC, MUL, EXT,
    // ZIP1/ZIP2) must all decode.
    let mut g = Guest::new();
    for word in [
        0x4e04_0e46, 0x4e22_1ca5, 0x4ea3_9ca5, 0x4ea4_4ca5, 0x6e05_40a7, 0x0e87_38b0, 0x0e91_38d2,
        0x0e87_78a5, 0x0e91_78c6,
    ] {
        g.exec(word);
    }
}
