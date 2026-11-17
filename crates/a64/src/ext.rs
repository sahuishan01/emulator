//! Advanced SIMD, extended scalar floating-point, CRC32 and EL0 system instructions.
//!
//! Every entry point returns "not handled" before touching CPU state when it does not
//! recognise an encoding, so the caller can report an explicit unsupported-instruction
//! fault. Cumulative saturation (FPSR.QC) and floating-point exception flags are not
//! modelled.

use crate::{Cpu, Nzcv};
use emulator_memory::{AddressSpace, MemoryError};

const LOW: u128 = u64::MAX as u128;

/// Counter frequency reported through `CNTFRQ_EL0` (Tegra X1 system tick rate).
pub const COUNTER_FREQUENCY_HZ: u64 = 19_200_000;

fn ones(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1_u64 << bits) - 1
    }
}

fn lane(v: u128, bits: u32, index: u32) -> u64 {
    ((v >> (bits * index)) as u64) & ones(bits)
}

fn put(v: &mut u128, bits: u32, index: u32, value: u64) {
    let shift = bits * index;
    *v = (*v & !(u128::from(ones(bits)) << shift)) | (u128::from(value & ones(bits)) << shift);
}

fn sx(value: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((value << shift) as i64) >> shift
}

fn build(bits: u32, lanes: u32, mut f: impl FnMut(u32) -> u64) -> u128 {
    let mut v = 0_u128;
    for index in 0..lanes {
        put(&mut v, bits, index, f(index));
    }
    v
}

fn try_build(bits: u32, lanes: u32, mut f: impl FnMut(u32) -> Option<u64>) -> Option<u128> {
    let mut v = 0_u128;
    for index in 0..lanes {
        put(&mut v, bits, index, f(index)?);
    }
    Some(v)
}

fn half(v: u128, upper: bool) -> u128 {
    if upper { v >> 64 } else { v & LOW }
}

fn narrow_into(old: u128, upper: bool, value: u128) -> u128 {
    if upper {
        (old & LOW) | ((value & LOW) << 64)
    } else {
        value & LOW
    }
}

fn mask_if(condition: bool, bits: u32) -> u64 {
    if condition { ones(bits) } else { 0 }
}

fn clamp_signed(value: i128, bits: u32) -> u64 {
    let max = (1_i128 << (bits - 1)) - 1;
    let min = -(1_i128 << (bits - 1));
    value.clamp(min, max) as u64 & ones(bits)
}

fn clamp_unsigned(value: i128, bits: u32) -> u64 {
    value.clamp(0, i128::from(ones(bits))) as u64
}

fn field(word: u32, low: u32, width: u32) -> u32 {
    (word >> low) & ((1 << width) - 1)
}

fn bit(word: u32, index: u32) -> bool {
    word & (1 << index) != 0
}

// ---------------------------------------------------------------------------
// Floating-point helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Round {
    Nearest,
    Plus,
    Minus,
    Zero,
    Away,
}

impl Round {
    fn from_rmode(rmode: u32) -> Self {
        match rmode & 3 {
            0 => Self::Nearest,
            1 => Self::Plus,
            2 => Self::Minus,
            _ => Self::Zero,
        }
    }

    fn from_fpcr(fpcr: u32) -> Self {
        Self::from_rmode(fpcr >> 22)
    }

    fn apply(self, value: f64) -> f64 {
        match self {
            Self::Nearest => value.round_ties_even(),
            Self::Plus => value.ceil(),
            Self::Minus => value.floor(),
            Self::Zero => value.trunc(),
            Self::Away => value.round(),
        }
    }
}

pub(crate) trait Fp: Copy {
    fn arm_max(self, other: Self) -> Self;
    fn arm_min(self, other: Self) -> Self;
    fn arm_maxnm(self, other: Self) -> Self;
    fn arm_minnm(self, other: Self) -> Self;
    fn arm_mulx(self, other: Self) -> Self;
    fn recip_step(self, other: Self) -> Self;
    fn rsqrt_step(self, other: Self) -> Self;
}

macro_rules! impl_fp {
    ($ty:ty) => {
        impl Fp for $ty {
            fn arm_max(self, other: Self) -> Self {
                if self.is_nan() || other.is_nan() {
                    self + other
                } else if self > other || (self == other && other.is_sign_negative()) {
                    self
                } else {
                    other
                }
            }
            fn arm_min(self, other: Self) -> Self {
                if self.is_nan() || other.is_nan() {
                    self + other
                } else if self < other || (self == other && self.is_sign_negative()) {
                    self
                } else {
                    other
                }
            }
            fn arm_maxnm(self, other: Self) -> Self {
                if self.is_nan() && !other.is_nan() {
                    other
                } else if other.is_nan() && !self.is_nan() {
                    self
                } else {
                    self.arm_max(other)
                }
            }
            fn arm_minnm(self, other: Self) -> Self {
                if self.is_nan() && !other.is_nan() {
                    other
                } else if other.is_nan() && !self.is_nan() {
                    self
                } else {
                    self.arm_min(other)
                }
            }
            fn arm_mulx(self, other: Self) -> Self {
                if (self == 0.0 && other.is_infinite()) || (self.is_infinite() && other == 0.0) {
                    if self.is_sign_negative() != other.is_sign_negative() {
                        -2.0
                    } else {
                        2.0
                    }
                } else {
                    self * other
                }
            }
            fn recip_step(self, other: Self) -> Self {
                if (self == 0.0 && other.is_infinite()) || (self.is_infinite() && other == 0.0) {
                    2.0
                } else {
                    (-self).mul_add(other, 2.0)
                }
            }
            fn rsqrt_step(self, other: Self) -> Self {
                if (self == 0.0 && other.is_infinite()) || (self.is_infinite() && other == 0.0) {
                    1.5
                } else {
                    (-self).mul_add(other, 3.0) / 2.0
                }
            }
        }
    };
}
impl_fp!(f32);
impl_fp!(f64);

/// Evaluates a float expression over one, two or three lane values in the lane precision.
macro_rules! fp {
    ($double:expr, |$($name:ident = $value:ident),+| $body:expr) => {
        if $double {
            $(let $name = f64::from_bits($value);)+
            let result: f64 = $body;
            result.to_bits()
        } else {
            $(let $name = f32::from_bits($value as u32);)+
            let result: f32 = $body;
            u64::from(result.to_bits())
        }
    };
}

/// Evaluates a float predicate over lane values in the lane precision.
macro_rules! fp_test {
    ($double:expr, |$($name:ident = $value:ident),+| $body:expr) => {
        if $double {
            $(let $name = f64::from_bits($value);)+
            $body
        } else {
            $(let $name = f32::from_bits($value as u32);)+
            $body
        }
    };
}

fn to_f64(double: bool, raw: u64) -> f64 {
    if double {
        f64::from_bits(raw)
    } else {
        f64::from(f32::from_bits(raw as u32))
    }
}

fn round_fp(double: bool, raw: u64, mode: Round) -> u64 {
    if double {
        mode.apply(f64::from_bits(raw)).to_bits()
    } else {
        // Rounding an f32 to an integral value is exact in f64 and back.
        u64::from((mode.apply(f64::from(f32::from_bits(raw as u32))) as f32).to_bits())
    }
}

/// Converts a float lane to a saturated integer of `int_bits` (32 or 64) with `fbits`
/// fractional bits. NaN converts to zero, as on hardware.
fn fp_to_int(double: bool, raw: u64, mode: Round, signed: bool, int_bits: u32, fbits: u32) -> u64 {
    let scaled = to_f64(double, raw) * 2.0_f64.powi(fbits as i32);
    let rounded = mode.apply(scaled);
    match (signed, int_bits) {
        (true, 64) => rounded as i64 as u64,
        (true, _) => u64::from(rounded as i32 as u32),
        (false, 64) => rounded as u64,
        (false, _) => u64::from(rounded as u32),
    }
}

/// Converts an integer of `int_bits` with `fbits` fractional bits to a float lane.
fn int_to_fp(double: bool, raw: u64, signed: bool, int_bits: u32, fbits: u32) -> u64 {
    let raw = raw & ones(int_bits);
    if double {
        let value = if signed {
            sx(raw, int_bits) as f64
        } else {
            raw as f64
        };
        (value / 2.0_f64.powi(fbits as i32)).to_bits()
    } else {
        let value = if signed {
            sx(raw, int_bits) as f32
        } else {
            raw as f32
        };
        u64::from((value / 2.0_f32.powi(fbits as i32)).to_bits())
    }
}

fn compare_flags(double: bool, a: u64, b: u64) -> Nzcv {
    let (a, b) = (to_f64(double, a), to_f64(double, b));
    let (negative, zero, carry, overflow) = if a.is_nan() || b.is_nan() {
        (false, false, true, true)
    } else if a == b {
        (false, true, true, false)
    } else if a < b {
        (true, false, false, false)
    } else {
        (false, false, true, false)
    };
    Nzcv {
        negative,
        zero,
        carry,
        overflow,
    }
}

fn expand_fp_imm(double: bool, imm8: u32) -> u64 {
    let sign = u64::from(imm8 >> 7) & 1;
    let b6 = u64::from(imm8 >> 6) & 1;
    let low = u64::from(imm8) & 0x3f;
    if double {
        (sign << 63) | ((b6 ^ 1) << 62) | ((b6 * 0xff) << 54) | (low << 48)
    } else {
        (sign << 31) | ((b6 ^ 1) << 30) | ((b6 * 0x1f) << 25) | (low << 19)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Executes `word` if it belongs to this module. Returns `Ok(false)` without changing
/// any state when the encoding is not handled here.
pub(crate) fn execute(
    cpu: &mut Cpu,
    memory: &mut AddressSpace,
    word: u32,
) -> Result<bool, MemoryError> {
    if (word >> 25) & 7 == 7 {
        if crate::crypto::execute(cpu, word).is_some() {
            return Ok(true);
        }
        return Ok(fp_simd(cpu, word).is_some());
    }
    if word & 0xbe00_0000 == 0x0c00_0000 {
        return simd_memory(cpu, memory, word);
    }
    if word & 0xffc0_0000 == 0xd500_0000 {
        return system(cpu, memory, word);
    }
    if word & 0x7fe0_e000 == 0x1ac0_4000 {
        return Ok(crc32(cpu, word).is_some());
    }
    Ok(false)
}

fn fp_simd(cpu: &mut Cpu, word: u32) -> Option<()> {
    if word & 0x7f00_0000 == 0x1e00_0000 {
        return scalar_fp(cpu, word);
    }
    let scalar = bit(word, 30) && bit(word, 28);
    let group = if scalar {
        word & 0xdf00_0000
    } else {
        word & 0x9f00_0000
    };
    match (scalar, group) {
        (false, 0x0e00_0000) | (true, 0x5e00_0000) => {
            if bit(word, 21) {
                match field(word, 10, 2) {
                    1 | 3 => three_same(cpu, word, scalar),
                    2 => match field(word, 17, 4) {
                        0 => two_reg(cpu, word, scalar),
                        8 if scalar => scalar_pairwise(cpu, word),
                        8 => across_lanes(cpu, word),
                        _ => None,
                    },
                    _ if scalar => None,
                    _ => three_different(cpu, word),
                }
            } else if scalar {
                if word & 0xffe0_fc00 == 0x5e00_0400 {
                    scalar_copy(cpu, word)
                } else {
                    None
                }
            } else if bit(word, 15) || field(word, 22, 2) != 0 {
                if !bit(word, 29) && !bit(word, 15) && field(word, 10, 2) == 2 {
                    permute(cpu, word)
                } else {
                    None
                }
            } else if bit(word, 10) {
                copy(cpu, word)
            } else if bit(word, 29) {
                extract(cpu, word)
            } else if field(word, 10, 2) == 0 {
                table(cpu, word)
            } else {
                permute(cpu, word)
            }
        }
        (false, 0x0f00_0000) | (true, 0x5f00_0000) => {
            if !bit(word, 10) {
                indexed(cpu, word, scalar)
            } else if field(word, 19, 4) != 0 {
                shift_immediate(cpu, word, scalar)
            } else if scalar {
                None
            } else {
                modified_immediate(cpu, word)
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Scalar floating point
// ---------------------------------------------------------------------------

fn scalar_fp(cpu: &mut Cpu, word: u32) -> Option<()> {
    let sf = bit(word, 31);
    let ty = field(word, 22, 2);
    let rm = field(word, 16, 5) as usize;
    let rn = field(word, 5, 5) as u8;
    let rd = field(word, 0, 5) as u8;
    let int_bits = if sf { 64 } else { 32 };

    if !bit(word, 21) {
        // Conversion between floating point and fixed point.
        if ty > 1 {
            return None;
        }
        let double = ty == 1;
        let scale = field(word, 10, 6);
        if !sf && scale < 32 {
            return None;
        }
        let fbits = 64 - scale;
        match (field(word, 19, 2), field(word, 16, 3)) {
            (0, opcode @ (2 | 3)) => {
                let raw = int_to_fp(double, cpu.read_zero(rn), opcode == 2, int_bits, fbits);
                cpu.v[usize::from(rd)] = u128::from(raw);
            }
            (3, opcode @ (0 | 1)) => {
                let raw = lane(cpu.v[usize::from(rn)], 64, 0);
                let value = fp_to_int(double, raw, Round::Zero, opcode == 0, int_bits, fbits);
                cpu.write_zero(rd, value);
            }
            _ => return None,
        }
        return Some(());
    }

    match field(word, 10, 2) {
        1 => {
            // FCCMP / FCCMPE
            if sf || ty > 1 {
                return None;
            }
            let condition = field(word, 12, 4) as u8;
            cpu.nzcv = if cpu.nzcv.holds(condition) {
                compare_flags(
                    ty == 1,
                    lane(cpu.v[usize::from(rn)], 64, 0),
                    lane(cpu.v[rm], 64, 0),
                )
            } else {
                Nzcv {
                    negative: bit(word, 3),
                    zero: bit(word, 2),
                    carry: bit(word, 1),
                    overflow: bit(word, 0),
                }
            };
            Some(())
        }
        3 => {
            // FCSEL
            if sf || ty > 1 {
                return None;
            }
            let condition = field(word, 12, 4) as u8;
            let source = if cpu.nzcv.holds(condition) {
                cpu.v[usize::from(rn)]
            } else {
                cpu.v[rm]
            };
            let bits = if ty == 1 { 64 } else { 32 };
            cpu.v[usize::from(rd)] = u128::from(lane(source, bits, 0));
            Some(())
        }
        0 if field(word, 10, 6) == 0 => fp_int_convert(cpu, word),
        0 if sf || ty > 1 => None,
        0 if field(word, 10, 5) == 0x10 => {
            // FRINT* (the remaining one-source operations are decoded by the caller).
            let double = ty == 1;
            let mode = match field(word, 15, 6) {
                8 => Round::Nearest,
                9 => Round::Plus,
                10 => Round::Minus,
                11 => Round::Zero,
                12 => Round::Away,
                14 | 15 => Round::from_fpcr(cpu.fpcr),
                _ => return None,
            };
            let raw = lane(cpu.v[usize::from(rn)], 64, 0);
            cpu.v[usize::from(rd)] = u128::from(round_fp(double, raw, mode));
            Some(())
        }
        0 if field(word, 10, 3) == 4 => {
            // FMOV (scalar, immediate)
            if field(word, 5, 5) != 0 {
                return None;
            }
            cpu.v[usize::from(rd)] = u128::from(expand_fp_imm(ty == 1, field(word, 13, 8)));
            Some(())
        }
        _ => None,
    }
}

fn fp_int_convert(cpu: &mut Cpu, word: u32) -> Option<()> {
    let sf = bit(word, 31);
    let ty = field(word, 22, 2);
    let rmode = field(word, 19, 2);
    let opcode = field(word, 16, 3);
    let rn = field(word, 5, 5) as u8;
    let rd = field(word, 0, 5) as u8;
    let int_bits = if sf { 64 } else { 32 };
    let double = ty == 1;
    match (ty, rmode, opcode) {
        (2, 1, 6) if sf => {
            let value = (cpu.v[usize::from(rn)] >> 64) as u64;
            cpu.write_zero(rd, value);
        }
        (2, 1, 7) if sf => {
            let value = cpu.read_zero(rn);
            let target = &mut cpu.v[usize::from(rd)];
            *target = (*target & LOW) | (u128::from(value) << 64);
        }
        (0 | 1, 0, 6) if sf == double => {
            let value = lane(cpu.v[usize::from(rn)], int_bits, 0);
            cpu.write_zero(rd, value);
        }
        (0 | 1, 0, 7) if sf == double => {
            cpu.v[usize::from(rd)] = u128::from(cpu.read_zero(rn) & ones(int_bits));
        }
        (0 | 1, 0, 2 | 3) => {
            let raw = int_to_fp(double, cpu.read_zero(rn), opcode == 2, int_bits, 0);
            cpu.v[usize::from(rd)] = u128::from(raw);
        }
        (0 | 1, _, 0 | 1) | (0 | 1, 0, 4 | 5) => {
            let mode = if opcode >= 4 {
                Round::Away
            } else {
                Round::from_rmode(rmode)
            };
            let raw = lane(cpu.v[usize::from(rn)], 64, 0);
            let value = fp_to_int(double, raw, mode, opcode & 1 == 0, int_bits, 0);
            cpu.write_zero(rd, value);
        }
        _ => return None,
    }
    Some(())
}

// ---------------------------------------------------------------------------
// AdvSIMD three-register, same type
// ---------------------------------------------------------------------------

fn int3(opcode: u32, u: bool, bits: u32, a: u64, b: u64, d: u64) -> Option<u64> {
    let wide = |value: u64| -> i128 {
        if u {
            i128::from(value)
        } else {
            i128::from(sx(value, bits))
        }
    };
    let (wa, wb) = (wide(a), wide(b));
    let saturate = |value: i128| -> u64 {
        if u {
            clamp_unsigned(value, bits)
        } else {
            clamp_signed(value, bits)
        }
    };
    let no_64 = matches!(opcode, 0 | 2 | 4 | 12..=15 | 18 | 19);
    if no_64 && bits == 64 {
        return None;
    }
    Some(match opcode {
        0 => ((wa + wb) >> 1) as u64,
        1 => saturate(wa + wb),
        2 => ((wa + wb + 1) >> 1) as u64,
        4 => ((wa - wb) >> 1) as u64,
        5 => saturate(wa - wb),
        6 => mask_if(wa > wb, bits),
        7 => mask_if(wa >= wb, bits),
        8 => {
            let shift = sx(b & 0xff, 8);
            if shift >= 0 {
                if shift >= i64::from(bits) {
                    0
                } else {
                    a << shift
                }
            } else {
                let right = (-shift) as u32;
                if u {
                    if right >= bits { 0 } else { a >> right }
                } else {
                    (sx(a, bits) >> right.min(63)) as u64
                }
            }
        }
        12 => (if wa > wb { wa } else { wb }) as u64,
        13 => (if wa < wb { wa } else { wb }) as u64,
        14 => (wa - wb).unsigned_abs() as u64,
        15 => d.wrapping_add((wa - wb).unsigned_abs() as u64),
        16 => {
            if u {
                a.wrapping_sub(b)
            } else {
                a.wrapping_add(b)
            }
        }
        17 => {
            if u {
                mask_if(a == b, bits)
            } else {
                mask_if(a & b != 0, bits)
            }
        }
        18 => {
            let product = a.wrapping_mul(b);
            if u {
                d.wrapping_sub(product)
            } else {
                d.wrapping_add(product)
            }
        }
        19 if !u => a.wrapping_mul(b),
        _ => return None,
    })
}

fn fp3(double: bool, u: bool, a: bool, opcode: u32, x: u64, y: u64, d: u64) -> Option<u64> {
    let bits = if double { 64 } else { 32 };
    Some(match (u, a, opcode) {
        (false, false, 24) => fp!(double, |p = x, q = y| p.arm_maxnm(q)),
        (false, true, 24) => fp!(double, |p = x, q = y| p.arm_minnm(q)),
        (false, false, 25) => fp!(double, |p = x, q = y, r = d| p.mul_add(q, r)),
        (false, true, 25) => fp!(double, |p = x, q = y, r = d| (-p).mul_add(q, r)),
        (false, false, 26) => fp!(double, |p = x, q = y| p + q),
        (false, true, 26) => fp!(double, |p = x, q = y| p - q),
        (false, false, 27) => fp!(double, |p = x, q = y| p.arm_mulx(q)),
        (false, false, 28) => mask_if(fp_test!(double, |p = x, q = y| p == q), bits),
        (false, false, 30) => fp!(double, |p = x, q = y| p.arm_max(q)),
        (false, true, 30) => fp!(double, |p = x, q = y| p.arm_min(q)),
        (false, false, 31) => fp!(double, |p = x, q = y| p.recip_step(q)),
        (false, true, 31) => fp!(double, |p = x, q = y| p.rsqrt_step(q)),
        (true, false, 27) => fp!(double, |p = x, q = y| p * q),
        (true, false, 28) => mask_if(fp_test!(double, |p = x, q = y| p >= q), bits),
        (true, false, 29) => mask_if(fp_test!(double, |p = x, q = y| p.abs() >= q.abs()), bits),
        (true, false, 31) => fp!(double, |p = x, q = y| p / q),
        (true, true, 26) => fp!(double, |p = x, q = y| (p - q).abs()),
        (true, true, 28) => mask_if(fp_test!(double, |p = x, q = y| p > q), bits),
        (true, true, 29) => mask_if(fp_test!(double, |p = x, q = y| p.abs() > q.abs()), bits),
        _ => return None,
    })
}

/// Pairwise floating-point reduction shared by vector and scalar forms.
/// `kind`: 24 = max/min number, 26 = add, 30 = max/min.
fn fp_pair(double: bool, min: bool, kind: u32, x: u64, y: u64) -> Option<u64> {
    Some(match (kind, min) {
        (24, false) => fp!(double, |p = x, q = y| p.arm_maxnm(q)),
        (24, true) => fp!(double, |p = x, q = y| p.arm_minnm(q)),
        (26, false) => fp!(double, |p = x, q = y| p + q),
        (30, false) => fp!(double, |p = x, q = y| p.arm_max(q)),
        (30, true) => fp!(double, |p = x, q = y| p.arm_min(q)),
        _ => return None,
    })
}

fn three_same(cpu: &mut Cpu, word: u32, scalar: bool) -> Option<()> {
    let q = bit(word, 30);
    let u = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 11, 5);
    let rd = field(word, 0, 5) as usize;
    let n = cpu.v[field(word, 5, 5) as usize];
    let m = cpu.v[field(word, 16, 5) as usize];
    let d = cpu.v[rd];
    let total = if q { 128 } else { 64 };

    let result = if opcode >= 24 {
        let double = size & 1 != 0;
        let a = size & 2 != 0;
        let bits = if double { 64 } else { 32 };
        if double && !q && !scalar {
            return None;
        }
        let lanes = if scalar { 1 } else { total / bits };
        let pairwise = u && matches!(opcode, 24 | 30) || (u && !a && opcode == 26);
        if pairwise {
            if scalar {
                return None;
            }
            let source = |index: u32| {
                if index < lanes {
                    lane(n, bits, index)
                } else {
                    lane(m, bits, index - lanes)
                }
            };
            try_build(bits, lanes, |i| {
                fp_pair(double, a, opcode, source(2 * i), source(2 * i + 1))
            })?
        } else {
            try_build(bits, lanes, |i| {
                fp3(
                    double,
                    u,
                    a,
                    opcode,
                    lane(n, bits, i),
                    lane(m, bits, i),
                    lane(d, bits, i),
                )
            })?
        }
    } else if opcode == 3 {
        if scalar {
            return None;
        }
        let value = match (u, size) {
            (false, 0) => n & m,
            (false, 1) => n & !m,
            (false, 2) => n | m,
            (false, _) => n | !m,
            (true, 0) => n ^ m,
            (true, 1) => (d & n) | (!d & m),
            (true, 2) => (d & !m) | (n & m),
            (true, _) => (d & m) | (n & !m),
        };
        if q { value } else { value & LOW }
    } else {
        let bits = 8 << size;
        if scalar && !(matches!(opcode, 1 | 5) || (matches!(opcode, 6 | 7 | 8 | 16 | 17) && size == 3))
        {
            return None;
        }
        if size == 3 && !q && !scalar {
            return None;
        }
        let lanes = if scalar { 1 } else { total / bits };
        if matches!(opcode, 20 | 21 | 23) {
            if size == 3 && opcode != 23 || (opcode == 23 && u) {
                return None;
            }
            let source = |index: u32| {
                if index < lanes {
                    lane(n, bits, index)
                } else {
                    lane(m, bits, index - lanes)
                }
            };
            build(bits, lanes, |i| {
                let (x, y) = (source(2 * i), source(2 * i + 1));
                match opcode {
                    20 => int3(12, u, bits, x, y, 0).unwrap_or(0),
                    21 => int3(13, u, bits, x, y, 0).unwrap_or(0),
                    _ => x.wrapping_add(y),
                }
            })
        } else {
            try_build(bits, lanes, |i| {
                int3(
                    opcode,
                    u,
                    bits,
                    lane(n, bits, i),
                    lane(m, bits, i),
                    lane(d, bits, i),
                )
            })?
        }
    };
    cpu.v[rd] = result;
    Some(())
}

// ---------------------------------------------------------------------------
// AdvSIMD two-register miscellaneous
// ---------------------------------------------------------------------------

fn reverse(v: u128, container: u32, bits: u32, total: u32) -> u128 {
    let per = container / bits;
    build(bits, total / bits, |i| {
        let base = i / per * per;
        lane(v, bits, base + (per - 1 - i % per))
    })
}

fn fp_two_reg(double: bool, a: bool, opcode: u32, u: bool, x: u64, fpcr: u32) -> Option<u64> {
    let bits = if double { 64 } else { 32 };
    let convert = |mode: Round| fp_to_int(double, x, mode, !u, bits, 0);
    Some(match (a, opcode, u) {
        (false, 24, false) => round_fp(double, x, Round::Nearest),
        (false, 24, true) => round_fp(double, x, Round::Away),
        (false, 25, false) => round_fp(double, x, Round::Minus),
        (false, 25, true) | (true, 24, true) => round_fp(double, x, Round::from_fpcr(fpcr)),
        (true, 24, false) => round_fp(double, x, Round::Plus),
        (true, 25, false) => round_fp(double, x, Round::Zero),
        (false, 26, _) => convert(Round::Nearest),
        (false, 27, _) => convert(Round::Minus),
        (false, 28, _) => convert(Round::Away),
        (true, 26, _) => convert(Round::Plus),
        (true, 27, _) => convert(Round::Zero),
        (false, 29, _) => int_to_fp(double, x, !u, bits, 0),
        (true, 12, false) => mask_if(fp_test!(double, |p = x| p > 0.0), bits),
        (true, 12, true) => mask_if(fp_test!(double, |p = x| p >= 0.0), bits),
        (true, 13, false) => mask_if(fp_test!(double, |p = x| p == 0.0), bits),
        (true, 13, true) => mask_if(fp_test!(double, |p = x| p <= 0.0), bits),
        (true, 14, false) => mask_if(fp_test!(double, |p = x| p < 0.0), bits),
        (true, 15, false) => x & !(1 << (bits - 1)),
        (true, 15, true) => x ^ (1 << (bits - 1)),
        (true, 31, true) => fp!(double, |p = x| p.sqrt()),
        (true, 29, false) => recip_estimate(double, x, fpcr),
        (true, 29, true) => rsqrt_estimate(double, x),
        _ => return None,
    })
}

/// Unpacked operand for the FRECPE/FRSQRTE estimate algorithms: sign, biased
/// exponent and a 52-bit fraction (single-precision fractions are widened).
fn estimate_parts(double: bool, x: u64) -> (u64, i64, u64, i64) {
    if double {
        (x >> 63, ((x >> 52) & 0x7ff) as i64, x & ((1 << 52) - 1), 0x7ff)
    } else {
        (x >> 31 & 1, ((x >> 23) & 0xff) as i64, (x & 0x7f_ffff) << 29, 0xff)
    }
}

fn estimate_pack(double: bool, sign: u64, exp: i64, fraction: u64) -> u64 {
    if double {
        sign << 63 | ((exp as u64) & 0x7ff) << 52 | fraction
    } else {
        sign << 31 | ((exp as u64) & 0xff) << 23 | fraction >> 29
    }
}

fn quiet_nan(double: bool, x: u64) -> u64 {
    x | if double { 1 << 51 } else { 1 << 22 }
}

/// FRECPE (Arm ARM FPRecipEstimate / RecipEstimate), FPCR.FZ not modelled.
fn recip_estimate(double: bool, x: u64, fpcr: u32) -> u64 {
    let (sign, mut exp, mut fraction, max_exp) = estimate_parts(double, x);
    if exp == max_exp {
        return if fraction != 0 { quiet_nan(double, x) } else { estimate_pack(double, sign, 0, 0) };
    }
    if exp == 0 && fraction == 0 {
        return estimate_pack(double, sign, max_exp, 0);
    }
    // |x| < 2^-128 (single) / 2^-1024 (double): the reciprocal overflows.
    if exp == 0 && fraction >> 50 == 0 {
        let rounding = (fpcr >> 22) & 3;
        let to_inf = match rounding {
            0 => true,
            1 => sign == 0,
            2 => sign == 1,
            _ => false,
        };
        return if to_inf {
            estimate_pack(double, sign, max_exp, 0)
        } else {
            estimate_pack(double, sign, max_exp - 1, (1 << 52) - 1)
        };
    }
    if exp == 0 {
        if fraction >> 51 & 1 == 0 {
            exp = -1;
            fraction = (fraction & ((1 << 50) - 1)) << 2;
        } else {
            fraction = (fraction & ((1 << 51) - 1)) << 1;
        }
    }
    let scaled = 0x100 | (fraction >> 44) & 0xff;
    let mut result_exp = if double { 2045 } else { 253 } - exp;
    let a = scaled * 2 + 1;
    let estimate = ((1u64 << 19) / a + 1) / 2;
    let mut fraction = (estimate & 0xff) << 44;
    if result_exp == 0 {
        fraction = 1 << 51 | fraction >> 1;
    } else if result_exp == -1 {
        fraction = 1 << 50 | fraction >> 2;
        result_exp = 0;
    }
    estimate_pack(double, sign, result_exp, fraction)
}

/// FRSQRTE (Arm ARM FPRSqrtEstimate / RecipSqrtEstimate), FPCR.FZ not modelled.
fn rsqrt_estimate(double: bool, x: u64) -> u64 {
    let (sign, mut exp, mut fraction, max_exp) = estimate_parts(double, x);
    let default_nan = if double { 0x7ff8_0000_0000_0000 } else { 0x7fc0_0000 };
    if exp == max_exp && fraction != 0 {
        return quiet_nan(double, x);
    }
    if exp == 0 && fraction == 0 {
        return estimate_pack(double, sign, max_exp, 0);
    }
    if sign == 1 {
        return default_nan;
    }
    if exp == max_exp {
        return 0;
    }
    if exp == 0 {
        while fraction >> 51 & 1 == 0 {
            fraction = (fraction << 1) & ((1 << 52) - 1);
            exp -= 1;
        }
        fraction = (fraction << 1) & ((1 << 52) - 1);
    }
    let scaled = if exp & 1 == 0 {
        0x100 | (fraction >> 44) & 0xff
    } else {
        0x80 | (fraction >> 45) & 0x7f
    };
    let result_exp = (if double { 3068 } else { 380 } - exp).div_euclid(2);
    let a = if scaled < 256 { scaled * 2 + 1 } else { (((scaled >> 1) << 1) + 1) * 2 };
    let mut b = 512u64;
    while a * (b + 1) * (b + 1) < 1 << 28 {
        b += 1;
    }
    let estimate = (b + 1) / 2;
    estimate_pack(double, 0, result_exp, (estimate & 0xff) << 44)
}

fn two_reg(cpu: &mut Cpu, word: u32, scalar: bool) -> Option<()> {
    let q = bit(word, 30);
    let u = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 5);
    let rd = field(word, 0, 5) as usize;
    let n = cpu.v[field(word, 5, 5) as usize];
    let d = cpu.v[rd];
    let total = if q { 128 } else { 64 };
    let bits = 8_u32 << size;

    let is_fp = matches!(opcode, 12..=15 | 22..=31);
    let result = if is_fp {
        let double = size & 1 != 0;
        let a = size & 2 != 0;
        let fbits = if double { 64 } else { 32 };
        match (a, opcode, u) {
            (false, 22, false) => {
                // FCVTN: double to single, narrowing.
                if !double || scalar {
                    return None;
                }
                let narrowed = build(32, 2, |i| {
                    u64::from((f64::from_bits(lane(n, 64, i)) as f32).to_bits())
                });
                narrow_into(d, q, narrowed)
            }
            (false, 23, false) => {
                // FCVTL: single to double, lengthening.
                if !double || scalar {
                    return None;
                }
                let source = half(n, q);
                build(64, 2, |i| {
                    f64::from(f32::from_bits(lane(source, 32, i) as u32)).to_bits()
                })
            }
            _ => {
                // Scalar AdvSIMD has conversions and compares only; rounding, FABS, FNEG
                // and FSQRT use the scalar floating-point encodings instead.
                if scalar && matches!(opcode, 15 | 24 | 25 | 31) {
                    return None;
                }
                if double && !q && !scalar {
                    return None;
                }
                let lanes = if scalar { 1 } else { total / fbits };
                let fpcr = cpu.fpcr;
                try_build(fbits, lanes, |i| {
                    fp_two_reg(double, a, opcode, u, lane(n, fbits, i), fpcr)
                })?
            }
        }
    } else if scalar {
        match (opcode, u) {
            (8..=11, _) if size == 3 && !(opcode == 10 && u) => {
                u128::from(int_two_reg(opcode, u, 64, lane(n, 64, 0))?)
            }
            (20, _) if size < 3 => u128::from(saturating_narrow(lane(n, 2 * bits, 0), bits, !u, !u)),
            // SQXTUN: signed source, unsigned saturated result.
            (18, true) if size < 3 => u128::from(saturating_narrow(lane(n, 2 * bits, 0), bits, true, false)),
            _ => return None,
        }
    } else {
        match (opcode, u) {
            (0, false) if size < 3 => reverse(n, 64, bits, total),
            (0, true) if size < 2 => reverse(n, 32, bits, total),
            (1, false) if size == 0 => reverse(n, 16, bits, total),
            (2 | 6, _) if size < 3 => build(2 * bits, total / bits / 2, |i| {
                let (x, y) = (lane(n, bits, 2 * i), lane(n, bits, 2 * i + 1));
                let sum = if u {
                    x + y
                } else {
                    (sx(x, bits) + sx(y, bits)) as u64
                };
                if opcode == 6 {
                    sum.wrapping_add(lane(d, 2 * bits, i))
                } else {
                    sum
                }
            }),
            (5, false) if size == 0 => build(8, total / 8, |i| u64::from(lane(n, 8, i).count_ones())),
            (5, true) if size == 0 => !n & if q { u128::MAX } else { LOW },
            (5, true) if size == 1 => build(8, total / 8, |i| {
                u64::from((lane(n, 8, i) as u8).reverse_bits())
            }),
            (18, false) if size < 3 => {
                narrow_into(d, q, build(bits, 64 / bits, |i| lane(n, 2 * bits, i)))
            }
            // SQXTUN{2}: signed source, unsigned saturated result.
            (18, true) if size < 3 => narrow_into(
                d,
                q,
                build(bits, 64 / bits, |i| saturating_narrow(lane(n, 2 * bits, i), bits, true, false)),
            ),
            (19, true) if size < 3 => {
                let source = half(n, q);
                build(2 * bits, 64 / bits, |i| lane(source, bits, i) << bits)
            }
            (20, _) if size < 3 => narrow_into(
                d,
                q,
                build(bits, 64 / bits, |i| {
                    saturating_narrow(lane(n, 2 * bits, i), bits, !u, !u)
                }),
            ),
            (4 | 8..=11, _) => {
                if size == 3 && (!q || opcode == 4) {
                    return None;
                }
                try_build(bits, total / bits, |i| {
                    int_two_reg(opcode, u, bits, lane(n, bits, i))
                })?
            }
            _ => return None,
        }
    };
    cpu.v[rd] = result;
    Some(())
}

fn int_two_reg(opcode: u32, u: bool, bits: u32, x: u64) -> Option<u64> {
    let signed = sx(x, bits);
    Some(match (opcode, u) {
        (4, false) => {
            let folded = (x ^ ((signed >> 1) as u64)) & ones(bits);
            leading_zeros(folded, bits) - 1
        }
        (4, true) => leading_zeros(x, bits),
        (8, false) => mask_if(signed > 0, bits),
        (8, true) => mask_if(signed >= 0, bits),
        (9, false) => mask_if(signed == 0, bits),
        (9, true) => mask_if(signed <= 0, bits),
        (10, false) => mask_if(signed < 0, bits),
        (11, false) => signed.wrapping_abs() as u64,
        (11, true) => signed.wrapping_neg() as u64,
        _ => return None,
    })
}

fn leading_zeros(x: u64, bits: u32) -> u64 {
    if x == 0 {
        u64::from(bits)
    } else {
        u64::from(x.leading_zeros() - (64 - bits))
    }
}

/// Narrows a `2 * bits` wide source lane to `bits` with saturation.
fn saturating_narrow(source: u64, bits: u32, source_signed: bool, result_signed: bool) -> u64 {
    let value = if source_signed {
        i128::from(sx(source, 2 * bits))
    } else {
        i128::from(source)
    };
    if result_signed {
        clamp_signed(value, bits)
    } else {
        clamp_unsigned(value, bits)
    }
}

fn across_lanes(cpu: &mut Cpu, word: u32) -> Option<()> {
    let q = bit(word, 30);
    let u = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 5);
    let rd = field(word, 0, 5) as usize;
    let n = cpu.v[field(word, 5, 5) as usize];
    let total = if q { 128 } else { 64 };

    let result = if matches!(opcode, 12 | 15) {
        // FMAXNMV / FMINNMV / FMAXV / FMINV on four single-precision lanes.
        if !u || !q || size & 1 != 0 {
            return None;
        }
        let min = size & 2 != 0;
        let kind = if opcode == 12 { 24 } else { 30 };
        let low = fp_pair(false, min, kind, lane(n, 32, 0), lane(n, 32, 1))?;
        let high = fp_pair(false, min, kind, lane(n, 32, 2), lane(n, 32, 3))?;
        fp_pair(false, min, kind, low, high)?
    } else {
        let bits = 8_u32 << size;
        let lanes = total / bits;
        if size == 3 || lanes < 4 {
            return None;
        }
        let values = (0..lanes).map(|i| {
            let raw = lane(n, bits, i);
            if u {
                i128::from(raw)
            } else {
                i128::from(sx(raw, bits))
            }
        });
        match (opcode, u) {
            (3, _) => values.sum::<i128>() as u64 & ones(2 * bits),
            (10, _) => values.max()? as u64 & ones(bits),
            (26, _) => values.min()? as u64 & ones(bits),
            (27, false) => values.sum::<i128>() as u64 & ones(bits),
            _ => return None,
        }
    };
    cpu.v[rd] = u128::from(result);
    Some(())
}

fn scalar_pairwise(cpu: &mut Cpu, word: u32) -> Option<()> {
    let u = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 5);
    let rd = field(word, 0, 5) as usize;
    let n = cpu.v[field(word, 5, 5) as usize];
    let result = match (opcode, u) {
        (27, false) if size == 3 => lane(n, 64, 0).wrapping_add(lane(n, 64, 1)),
        (12 | 13 | 15, true) => {
            let double = size & 1 != 0;
            let min = size & 2 != 0;
            let bits = if double { 64 } else { 32 };
            let kind = match opcode {
                12 => 24,
                13 => 26,
                _ => 30,
            };
            fp_pair(double, min, kind, lane(n, bits, 0), lane(n, bits, 1))?
        }
        _ => return None,
    };
    cpu.v[rd] = u128::from(result);
    Some(())
}

// ---------------------------------------------------------------------------
// AdvSIMD copy, permute, extract, table lookup
// ---------------------------------------------------------------------------

fn copy(cpu: &mut Cpu, word: u32) -> Option<()> {
    let q = bit(word, 30);
    let imm5 = field(word, 16, 5);
    let imm4 = field(word, 11, 4);
    let rn = field(word, 5, 5) as u8;
    let rd = field(word, 0, 5) as u8;
    let size = imm5.trailing_zeros();
    if size > 3 {
        return None;
    }
    let bits = 8_u32 << size;
    let index = imm5 >> (size + 1);
    let total = if q { 128 } else { 64 };
    let n = cpu.v[usize::from(rn)];

    if bit(word, 29) {
        // INS (element)
        if !q {
            return None;
        }
        let value = lane(n, bits, imm4 >> size);
        put(&mut cpu.v[usize::from(rd)], bits, index, value);
        return Some(());
    }
    match imm4 {
        0 | 1 => {
            if size == 3 && !q {
                return None;
            }
            let value = if imm4 == 0 {
                lane(n, bits, index)
            } else {
                cpu.read_zero(rn)
            };
            cpu.v[usize::from(rd)] = build(bits, total / bits, |_| value);
        }
        3 if q => {
            let value = cpu.read_zero(rn);
            put(&mut cpu.v[usize::from(rd)], bits, index, value);
        }
        5 => {
            // SMOV
            let target = if q { 64 } else { 32 };
            if bits >= target {
                return None;
            }
            cpu.write_zero(rd, sx(lane(n, bits, index), bits) as u64 & ones(target));
        }
        7 => {
            // UMOV
            if q != (size == 3) {
                return None;
            }
            cpu.write_zero(rd, lane(n, bits, index));
        }
        _ => return None,
    }
    Some(())
}

fn scalar_copy(cpu: &mut Cpu, word: u32) -> Option<()> {
    let imm5 = field(word, 16, 5);
    let size = imm5.trailing_zeros();
    if size > 3 {
        return None;
    }
    let bits = 8_u32 << size;
    let n = cpu.v[field(word, 5, 5) as usize];
    cpu.v[field(word, 0, 5) as usize] = u128::from(lane(n, bits, imm5 >> (size + 1)));
    Some(())
}

fn permute(cpu: &mut Cpu, word: u32) -> Option<()> {
    let q = bit(word, 30);
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 3);
    let n = cpu.v[field(word, 5, 5) as usize];
    let m = cpu.v[field(word, 16, 5) as usize];
    if size == 3 && !q {
        return None;
    }
    let bits = 8_u32 << size;
    let lanes = (if q { 128 } else { 64 }) / bits;
    let part = opcode >> 2;
    let result = match opcode & 3 {
        1 => build(bits, lanes, |i| {
            let index = 2 * i + part;
            if index < lanes {
                lane(n, bits, index)
            } else {
                lane(m, bits, index - lanes)
            }
        }),
        2 => build(bits, lanes, |i| {
            let source = if i & 1 == 0 { n } else { m };
            lane(source, bits, (i & !1) + part)
        }),
        3 => build(bits, lanes, |i| {
            let source = if i & 1 == 0 { n } else { m };
            lane(source, bits, part * (lanes / 2) + i / 2)
        }),
        _ => return None,
    };
    cpu.v[field(word, 0, 5) as usize] = result;
    Some(())
}

fn extract(cpu: &mut Cpu, word: u32) -> Option<()> {
    let q = bit(word, 30);
    let imm4 = field(word, 11, 4);
    let n = cpu.v[field(word, 5, 5) as usize];
    let m = cpu.v[field(word, 16, 5) as usize];
    let shift = imm4 * 8;
    let result = if q {
        if shift == 0 {
            n
        } else {
            (n >> shift) | (m << (128 - shift))
        }
    } else {
        if imm4 >= 8 {
            return None;
        }
        (((n & LOW) | ((m & LOW) << 64)) >> shift) & LOW
    };
    cpu.v[field(word, 0, 5) as usize] = result;
    Some(())
}

fn table(cpu: &mut Cpu, word: u32) -> Option<()> {
    let q = bit(word, 30);
    let rn = field(word, 5, 5);
    let rd = field(word, 0, 5) as usize;
    let registers = field(word, 13, 2) + 1;
    let extend = bit(word, 12);
    let indices = cpu.v[field(word, 16, 5) as usize];
    let d = cpu.v[rd];
    let result = build(8, if q { 16 } else { 8 }, |i| {
        let index = lane(indices, 8, i) as u32;
        if index < 16 * registers {
            let register = ((rn + index / 16) % 32) as usize;
            lane(cpu.v[register], 8, index % 16)
        } else if extend {
            lane(d, 8, i)
        } else {
            0
        }
    });
    cpu.v[rd] = result;
    Some(())
}

// ---------------------------------------------------------------------------
// AdvSIMD three-register, different widths
// ---------------------------------------------------------------------------

fn three_different(cpu: &mut Cpu, word: u32) -> Option<()> {
    let q = bit(word, 30);
    let u = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 4);
    let rd = field(word, 0, 5) as usize;
    let n = cpu.v[field(word, 5, 5) as usize];
    let m = cpu.v[field(word, 16, 5) as usize];
    let d = cpu.v[rd];
    if size == 3 {
        return None;
    }
    let bits = 8_u32 << size;
    let wide = 2 * bits;
    let lanes = 64 / bits;
    let extend = |value: u64| -> i128 {
        if u {
            i128::from(value)
        } else {
            i128::from(sx(value, bits))
        }
    };
    let (hn, hm) = (half(n, q), half(m, q));
    let narrow = |i: u32| extend(lane(hn, bits, i));
    let other = |i: u32| extend(lane(hm, bits, i));
    let result = match opcode {
        0 => build(wide, lanes, |i| (narrow(i) + other(i)) as u64),
        1 => build(wide, lanes, |i| lane(n, wide, i).wrapping_add(other(i) as u64)),
        2 => build(wide, lanes, |i| (narrow(i) - other(i)) as u64),
        3 => build(wide, lanes, |i| lane(n, wide, i).wrapping_sub(other(i) as u64)),
        4 | 6 => {
            let round = if u { 1_u64 << (bits - 1) } else { 0 };
            let value = build(bits, lanes, |i| {
                let (x, y) = (lane(n, wide, i), lane(m, wide, i));
                let combined = if opcode == 4 {
                    x.wrapping_add(y)
                } else {
                    x.wrapping_sub(y)
                };
                (combined.wrapping_add(round) & ones(wide)) >> bits
            });
            narrow_into(d, q, value)
        }
        5 => build(wide, lanes, |i| {
            lane(d, wide, i).wrapping_add((narrow(i) - other(i)).unsigned_abs() as u64)
        }),
        7 => build(wide, lanes, |i| (narrow(i) - other(i)).unsigned_abs() as u64),
        8 => build(wide, lanes, |i| {
            lane(d, wide, i).wrapping_add((narrow(i) * other(i)) as u64)
        }),
        10 => build(wide, lanes, |i| {
            lane(d, wide, i).wrapping_sub((narrow(i) * other(i)) as u64)
        }),
        12 => build(wide, lanes, |i| (narrow(i) * other(i)) as u64),
        _ => return None,
    };
    cpu.v[rd] = result;
    Some(())
}

// ---------------------------------------------------------------------------
// AdvSIMD shift by immediate, modified immediate, indexed element
// ---------------------------------------------------------------------------

fn shift_right(value: u64, bits: u32, shift: u32, signed: bool, round: bool) -> u64 {
    let wide = if signed {
        i128::from(sx(value, bits))
    } else {
        i128::from(value)
    };
    let rounding = if round { 1_i128 << (shift - 1) } else { 0 };
    ((wide + rounding) >> shift) as u64 & ones(bits)
}

fn shift_immediate(cpu: &mut Cpu, word: u32, scalar: bool) -> Option<()> {
    let q = bit(word, 30);
    let u = bit(word, 29);
    let immh = field(word, 19, 4);
    let imm = field(word, 16, 7);
    let opcode = field(word, 11, 5);
    let rd = field(word, 0, 5) as usize;
    let n = cpu.v[field(word, 5, 5) as usize];
    let d = cpu.v[rd];
    let bits = 8_u32 << (31 - immh.leading_zeros());
    let total = if q { 128 } else { 64 };

    let result = match opcode {
        0 | 2 | 4 | 6 | 8 | 10 => {
            if scalar && bits != 64 || (bits == 64 && !q && !scalar) {
                return None;
            }
            if opcode == 8 && !u {
                return None;
            }
            let lanes = if scalar { 1 } else { total / bits };
            let right = 2 * bits - imm;
            let left = imm - bits;
            build(bits, lanes, |i| {
                let (x, old) = (lane(n, bits, i), lane(d, bits, i));
                match opcode {
                    0 => shift_right(x, bits, right, !u, false),
                    2 => old.wrapping_add(shift_right(x, bits, right, !u, false)),
                    4 => shift_right(x, bits, right, !u, true),
                    6 => old.wrapping_add(shift_right(x, bits, right, !u, true)),
                    8 => {
                        let mask = if right >= bits { 0 } else { ones(bits) >> right };
                        (old & !mask) | (shift_right(x, bits, right, false, false) & mask)
                    }
                    _ => {
                        let shifted = x << left;
                        if u {
                            let mask = ones(bits) << left;
                            (old & !mask) | (shifted & mask)
                        } else {
                            shifted
                        }
                    }
                }
            })
        }
        16..=19 => {
            // Narrowing right shifts: SHRN, RSHRN, SQSHRUN, SQRSHRUN, (U)QSHRN, (U)QRSHRN.
            if bits == 64 {
                return None;
            }
            let right = 2 * bits - imm;
            let round = opcode & 1 != 0;
            let saturating = opcode >= 18 || u;
            let source_signed = !(opcode >= 18 && u);
            let result_signed = opcode >= 18 && !u;
            let narrow = |i: u32| {
                let source = lane(n, 2 * bits, i);
                if saturating {
                    let shifted = shift_right(source, 2 * bits, right, source_signed, round);
                    saturating_narrow(shifted, bits, source_signed, result_signed)
                } else {
                    shift_right(source, 2 * bits, right, false, round)
                }
            };
            if scalar {
                if !saturating {
                    return None;
                }
                u128::from(narrow(0))
            } else {
                narrow_into(d, q, build(bits, 64 / bits, narrow))
            }
        }
        20 => {
            // SSHLL / USHLL
            if scalar || bits == 64 {
                return None;
            }
            let left = imm - bits;
            let source = half(n, q);
            build(2 * bits, 64 / bits, |i| {
                let raw = lane(source, bits, i);
                let extended = if u { raw } else { sx(raw, bits) as u64 };
                extended << left
            })
        }
        28 | 31 => {
            // SCVTF / UCVTF / FCVTZS / FCVTZU with fractional bits.
            if bits < 32 || (bits == 64 && !q && !scalar) {
                return None;
            }
            let double = bits == 64;
            let fbits = 2 * bits - imm;
            let lanes = if scalar { 1 } else { total / bits };
            build(bits, lanes, |i| {
                let x = lane(n, bits, i);
                if opcode == 28 {
                    int_to_fp(double, x, !u, bits, fbits)
                } else {
                    fp_to_int(double, x, Round::Zero, !u, bits, fbits)
                }
            })
        }
        _ => return None,
    };
    cpu.v[rd] = result;
    Some(())
}

fn modified_immediate(cpu: &mut Cpu, word: u32) -> Option<()> {
    let q = bit(word, 30);
    let op = bit(word, 29);
    let cmode = field(word, 12, 4);
    if bit(word, 11) {
        return None;
    }
    let imm8 = u64::from((field(word, 16, 3) << 5) | field(word, 5, 5));
    let rd = field(word, 0, 5) as usize;
    let replicate32 = |value: u64| (value << 32) | value;
    let replicate16 = |value: u64| value * 0x0001_0001_0001_0001;

    enum Action {
        Move,
        MoveNot,
        Or,
        Clear,
    }
    let (imm64, action) = match cmode {
        0b0000 | 0b0010 | 0b0100 | 0b0110 | 0b0001 | 0b0011 | 0b0101 | 0b0111 => {
            let value = replicate32(imm8 << (8 * (cmode >> 1)));
            let action = match (cmode & 1 != 0, op) {
                (false, false) => Action::Move,
                (false, true) => Action::MoveNot,
                (true, false) => Action::Or,
                (true, true) => Action::Clear,
            };
            (value, action)
        }
        0b1000 | 0b1010 | 0b1001 | 0b1011 => {
            let value = replicate16(imm8 << (8 * ((cmode >> 1) & 1)));
            let action = match (cmode & 1 != 0, op) {
                (false, false) => Action::Move,
                (false, true) => Action::MoveNot,
                (true, false) => Action::Or,
                (true, true) => Action::Clear,
            };
            (value, action)
        }
        0b1100 => (
            replicate32((imm8 << 8) | 0xff),
            if op { Action::MoveNot } else { Action::Move },
        ),
        0b1101 => (
            replicate32((imm8 << 16) | 0xffff),
            if op { Action::MoveNot } else { Action::Move },
        ),
        0b1110 if !op => (imm8 * 0x0101_0101_0101_0101, Action::Move),
        0b1110 => {
            let mut value = 0_u64;
            for index in 0..8 {
                if imm8 & (1 << index) != 0 {
                    value |= 0xff << (8 * index);
                }
            }
            (value, Action::Move)
        }
        _ if !op => (
            replicate32(expand_fp_imm(false, imm8 as u32)),
            Action::Move,
        ),
        _ => {
            if !q {
                return None;
            }
            (expand_fp_imm(true, imm8 as u32), Action::Move)
        }
    };
    let full = if q {
        (u128::from(imm64) << 64) | u128::from(imm64)
    } else {
        u128::from(imm64)
    };
    let keep = if q { u128::MAX } else { LOW };
    let d = cpu.v[rd];
    cpu.v[rd] = match action {
        Action::Move => full,
        Action::MoveNot => !full & keep,
        Action::Or => (d | full) & keep,
        Action::Clear => (d & !full) & keep,
    };
    Some(())
}

fn indexed(cpu: &mut Cpu, word: u32, scalar: bool) -> Option<()> {
    let q = bit(word, 30);
    let u = bit(word, 29);
    let size = field(word, 22, 2);
    let l = field(word, 21, 1);
    let m = field(word, 20, 1);
    let rm4 = field(word, 16, 4);
    let opcode = field(word, 12, 4);
    let h = field(word, 11, 1);
    let rd = field(word, 0, 5) as usize;
    let n = cpu.v[field(word, 5, 5) as usize];
    let d = cpu.v[rd];
    let total = if q { 128 } else { 64 };

    let is_fp = matches!((u, opcode), (false, 1 | 5 | 9) | (true, 9));
    let result = if is_fp {
        if size < 2 {
            return None;
        }
        let double = size == 3;
        if double && (l != 0 || (!q && !scalar)) {
            return None;
        }
        let index = if double { h } else { (h << 1) | l };
        let bits = if double { 64 } else { 32 };
        let element = lane(cpu.v[((m << 4) | rm4) as usize], bits, index);
        let lanes = if scalar { 1 } else { total / bits };
        build(bits, lanes, |i| {
            let (x, old) = (lane(n, bits, i), lane(d, bits, i));
            match (u, opcode) {
                (false, 1) => fp!(double, |p = x, e = element, r = old| p.mul_add(e, r)),
                (false, 5) => fp!(double, |p = x, e = element, r = old| (-p).mul_add(e, r)),
                (false, _) => fp!(double, |p = x, e = element| p * e),
                (true, _) => fp!(double, |p = x, e = element| p.arm_mulx(e)),
            }
        })
    } else {
        if scalar {
            return None;
        }
        let (index, rm) = match size {
            1 => ((h << 2) | (l << 1) | m, rm4),
            2 => ((h << 1) | l, (m << 4) | rm4),
            _ => return None,
        };
        let bits = 8_u32 << size;
        let element = lane(cpu.v[rm as usize], bits, index);
        let extend = |value: u64| -> i128 {
            if u {
                i128::from(value)
            } else {
                i128::from(sx(value, bits))
            }
        };
        match (u, opcode) {
            (false, 8) => build(bits, total / bits, |i| lane(n, bits, i).wrapping_mul(element)),
            (true, 0) => build(bits, total / bits, |i| {
                lane(d, bits, i).wrapping_add(lane(n, bits, i).wrapping_mul(element))
            }),
            (true, 4) => build(bits, total / bits, |i| {
                lane(d, bits, i).wrapping_sub(lane(n, bits, i).wrapping_mul(element))
            }),
            (_, 2 | 6 | 10) => {
                let source = half(n, q);
                build(2 * bits, 64 / bits, |i| {
                    let product = (extend(lane(source, bits, i)) * extend(element)) as u64;
                    let old = lane(d, 2 * bits, i);
                    match opcode {
                        2 => old.wrapping_add(product),
                        6 => old.wrapping_sub(product),
                        _ => product,
                    }
                })
            }
            _ => return None,
        }
    };
    cpu.v[rd] = result;
    Some(())
}

// ---------------------------------------------------------------------------
// AdvSIMD structure loads and stores
// ---------------------------------------------------------------------------

fn read_element(memory: &AddressSpace, address: u64, bytes: u32) -> Result<u64, MemoryError> {
    Ok(match bytes {
        1 => u64::from(memory.read_u8(address)?),
        2 => u64::from(memory.read_u16(address)?),
        4 => u64::from(memory.read_u32(address)?),
        _ => memory.read_u64(address)?,
    })
}

fn write_element(
    memory: &mut AddressSpace,
    address: u64,
    bytes: u32,
    value: u64,
) -> Result<(), MemoryError> {
    match bytes {
        1 => memory.write_u8(address, value as u8),
        2 => memory.write_u16(address, value as u16),
        4 => memory.write_u32(address, value as u32),
        _ => memory.write_u64(address, value),
    }
}

fn simd_memory(
    cpu: &mut Cpu,
    memory: &mut AddressSpace,
    word: u32,
) -> Result<bool, MemoryError> {
    let q = bit(word, 30);
    let single = bit(word, 24);
    let post = bit(word, 23);
    let load = bit(word, 22);
    let rm = field(word, 16, 5) as u8;
    let rn = field(word, 5, 5) as u8;
    let rt = field(word, 0, 5);
    if !post && rm != 0 {
        return Ok(false);
    }
    let base = cpu.read_sp(rn);
    let total_bits = if q { 128 } else { 64 };
    let mut loaded = [0_u128; 4];
    let registers;
    let total_bytes;

    if single {
        let opcode = field(word, 13, 3);
        let s = field(word, 12, 1);
        let size = field(word, 10, 2);
        let qbit = u32::from(q);
        registers = (((opcode & 1) << 1) | field(word, 21, 1)) + 1;
        let (log2, index, replicate) = match opcode >> 1 {
            0 => (0, (qbit << 3) | (s << 2) | size, false),
            1 if size & 1 == 0 => (1, (qbit << 2) | (s << 1) | (size >> 1), false),
            2 if size == 0 => (2, (qbit << 1) | s, false),
            2 if size == 1 && s == 0 => (3, qbit, false),
            3 if load && s == 0 => (size, 0, true),
            _ => return Ok(false),
        };
        let bytes = 1_u32 << log2;
        let bits = 8 * bytes;
        for k in 0..registers {
            let register = ((rt + k) % 32) as usize;
            let address = base.wrapping_add(u64::from(k * bytes));
            if load {
                let value = read_element(memory, address, bytes)?;
                loaded[k as usize] = if replicate {
                    build(bits, total_bits / bits, |_| value)
                } else {
                    let mut current = cpu.v[register];
                    put(&mut current, bits, index, value);
                    current
                };
            } else {
                write_element(memory, address, bytes, lane(cpu.v[register], bits, index))?;
            }
        }
        total_bytes = registers * bytes;
    } else {
        if bit(word, 21) {
            return Ok(false);
        }
        let size = field(word, 10, 2);
        let (repeat, elements) = match field(word, 12, 4) {
            0 => (1, 4),
            2 => (4, 1),
            4 => (1, 3),
            6 => (3, 1),
            7 => (1, 1),
            8 => (1, 2),
            10 => (2, 1),
            _ => return Ok(false),
        };
        if elements > 1 && size == 3 && !q {
            return Ok(false);
        }
        let bytes = 1_u32 << size;
        let bits = 8 * bytes;
        let lanes = total_bits / bits;
        registers = repeat * elements;
        let mut address = base;
        for r in 0..repeat {
            for e in 0..lanes {
                for s in 0..elements {
                    let slot = r + s;
                    if load {
                        let value = read_element(memory, address, bytes)?;
                        put(&mut loaded[slot as usize], bits, e, value);
                    } else {
                        let register = ((rt + slot) % 32) as usize;
                        write_element(memory, address, bytes, lane(cpu.v[register], bits, e))?;
                    }
                    address = address.wrapping_add(u64::from(bytes));
                }
            }
        }
        total_bytes = registers * lanes * bytes;
    }

    if load {
        for k in 0..registers {
            cpu.v[((rt + k) % 32) as usize] = loaded[k as usize];
        }
    }
    if post {
        let offset = if rm == 31 {
            u64::from(total_bytes)
        } else {
            cpu.read_zero(rm)
        };
        cpu.write_sp(rn, base.wrapping_add(offset));
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// System registers, cache operations and CRC32
// ---------------------------------------------------------------------------

const fn sysreg(op1: u32, crn: u32, crm: u32, op2: u32) -> u32 {
    0x4000 | (op1 << 11) | (crn << 7) | (crm << 3) | op2
}

const CTR_EL0: u32 = sysreg(3, 0, 0, 1);
const DCZID_EL0: u32 = sysreg(3, 0, 0, 7);
const NZCV: u32 = sysreg(3, 4, 2, 0);
const FPCR: u32 = sysreg(3, 4, 4, 0);
const FPSR: u32 = sysreg(3, 4, 4, 1);
const TPIDR_EL0: u32 = sysreg(3, 13, 0, 2);
const TPIDRRO_EL0: u32 = sysreg(3, 13, 0, 3);
const CNTFRQ_EL0: u32 = sysreg(3, 14, 0, 0);
const CNTPCT_EL0: u32 = sysreg(3, 14, 0, 1);
const CNTVCT_EL0: u32 = sysreg(3, 14, 0, 2);

/// Cortex-A57 cache type: 64-byte instruction and data lines.
const CTR_EL0_VALUE: u64 = 0x8444_c004;
/// `DC ZVA` is permitted and clears 64 bytes.
const DCZID_EL0_VALUE: u64 = 4;
const ZVA_BYTES: u64 = 64;

fn system(cpu: &mut Cpu, memory: &mut AddressSpace, word: u32) -> Result<bool, MemoryError> {
    let rt = field(word, 0, 5) as u8;
    match word & 0xffff_ffe0 {
        0xd50b_7420 => {
            // DC ZVA
            let address = cpu.read_zero(rt) & !(ZVA_BYTES - 1);
            memory.write(address, &[0_u8; ZVA_BYTES as usize])?;
            return Ok(true);
        }
        // DC CVAC / CVAU / CIVAC and IC IVAU: no cache is modelled, so these have no effect.
        0xd50b_7a20 | 0xd50b_7b20 | 0xd50b_7e20 | 0xd50b_7520 => return Ok(true),
        _ => {}
    }
    let register = field(word, 5, 15);
    match word & 0xfff0_0000 {
        0xd530_0000 => {
            let value = match register {
                CTR_EL0 => CTR_EL0_VALUE,
                DCZID_EL0 => DCZID_EL0_VALUE,
                NZCV => {
                    (u64::from(cpu.nzcv.negative) << 31)
                        | (u64::from(cpu.nzcv.zero) << 30)
                        | (u64::from(cpu.nzcv.carry) << 29)
                        | (u64::from(cpu.nzcv.overflow) << 28)
                }
                FPCR => u64::from(cpu.fpcr),
                FPSR => u64::from(cpu.fpsr),
                TPIDR_EL0 => cpu.tpidr_el0,
                TPIDRRO_EL0 => cpu.tpidrro_el0,
                CNTFRQ_EL0 => COUNTER_FREQUENCY_HZ,
                CNTPCT_EL0 | CNTVCT_EL0 => cpu.counter,
                _ => return Ok(false),
            };
            cpu.write_zero(rt, value);
            Ok(true)
        }
        0xd510_0000 => {
            let value = cpu.read_zero(rt);
            match register {
                NZCV => {
                    cpu.nzcv = Nzcv {
                        negative: value & (1 << 31) != 0,
                        zero: value & (1 << 30) != 0,
                        carry: value & (1 << 29) != 0,
                        overflow: value & (1 << 28) != 0,
                    };
                }
                FPCR => cpu.fpcr = value as u32,
                FPSR => cpu.fpsr = value as u32,
                TPIDR_EL0 => cpu.tpidr_el0 = value,
                _ => return Ok(false),
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn crc32(cpu: &mut Cpu, word: u32) -> Option<()> {
    let sf = bit(word, 31);
    let size = field(word, 10, 2);
    if sf != (size == 3) {
        return None;
    }
    let polynomial: u64 = if bit(word, 12) {
        0x82f6_3b78
    } else {
        0xedb8_8320
    };
    let bits = 8_u32 << size;
    let rm = field(word, 16, 5) as u8;
    let rn = field(word, 5, 5) as u8;
    let rd = field(word, 0, 5) as u8;
    let mut state = (cpu.read_zero(rn) & 0xffff_ffff) ^ (cpu.read_zero(rm) & ones(bits));
    for _ in 0..bits {
        state = if state & 1 != 0 {
            (state >> 1) ^ polynomial
        } else {
            state >> 1
        };
    }
    cpu.write_zero(rd, state & 0xffff_ffff);
    Some(())
}
