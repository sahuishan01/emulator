//! Fixed-point primitives with the exact rounding of the SILK reference
//! arithmetic (RFC 6716 section 4.2 is defined in these terms).

#![allow(dead_code)]

#[inline(always)]
pub fn smulwb(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b as i16)) >> 16) as i32
}
#[inline(always)]
pub fn smlawb(a: i32, b: i32, c: i32) -> i32 {
    a.wrapping_add(((i64::from(b) * i64::from(c as i16)) >> 16) as i32)
}
#[inline(always)]
pub fn smulwt(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b >> 16)) >> 16) as i32
}
#[inline(always)]
pub fn smulbb(a: i32, b: i32) -> i32 {
    i32::from(a as i16) * i32::from(b as i16)
}
#[inline(always)]
pub fn smlabb(a: i32, b: i32, c: i32) -> i32 {
    a.wrapping_add(i32::from(b as i16) * i32::from(c as i16))
}
#[inline(always)]
pub fn smultt(a: i32, b: i32) -> i32 {
    (a >> 16) * (b >> 16)
}
#[inline(always)]
pub fn smulww(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}
#[inline(always)]
pub fn smlaww(a: i32, b: i32, c: i32) -> i32 {
    a.wrapping_add(((i64::from(b) * i64::from(c)) >> 16) as i32)
}
#[inline(always)]
pub fn smmul(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 32) as i32
}
#[inline(always)]
pub fn rshift_round(a: i32, shift: u32) -> i32 {
    if shift == 1 {
        (a >> 1) + (a & 1)
    } else {
        ((a >> (shift - 1)) + 1) >> 1
    }
}
#[inline(always)]
pub fn rshift_round64(a: i64, shift: u32) -> i64 {
    if shift == 1 {
        (a >> 1) + (a & 1)
    } else {
        ((a >> (shift - 1)) + 1) >> 1
    }
}
#[inline(always)]
pub fn sat16(a: i32) -> i32 {
    a.clamp(i32::from(i16::MIN), i32::from(i16::MAX))
}
#[inline(always)]
pub fn add_sat32(a: i32, b: i32) -> i32 {
    a.saturating_add(b)
}
#[inline(always)]
pub fn sub_sat32(a: i32, b: i32) -> i32 {
    a.saturating_sub(b)
}
#[inline(always)]
pub fn lshift_sat32(a: i32, shift: u32) -> i32 {
    let lo = i32::MIN >> shift;
    let hi = i32::MAX >> shift;
    a.clamp(lo, hi) << shift
}
#[inline(always)]
pub fn clz32(a: i32) -> i32 {
    (a as u32).leading_zeros() as i32
}
#[inline(always)]
pub fn rand(seed: i32) -> i32 {
    907_633_515i32.wrapping_add(seed.wrapping_mul(196_314_165))
}
/// `silk_LIMIT` with either bound order.
#[inline(always)]
pub fn limit(a: i32, l1: i32, l2: i32) -> i32 {
    if l1 > l2 {
        if a > l1 {
            l1
        } else if a < l2 {
            l2
        } else {
            a
        }
    } else if a > l2 {
        l2
    } else if a < l1 {
        l1
    } else {
        a
    }
}

#[inline]
fn ror32(a: u32, rot: i32) -> u32 {
    let m = (-rot) as u32;
    if rot == 0 {
        a
    } else if rot < 0 {
        (a << m) | (a >> (32 - m))
    } else {
        (a << (32 - rot as u32)) | (a >> rot as u32)
    }
}

/// Leading zeros and the 7 bits after the leading one.
pub fn clz_frac(x: i32) -> (i32, i32) {
    let lz = clz32(x);
    let frac = (ror32(x as u32, 24 - lz) & 0x7f) as i32;
    (lz, frac)
}

pub fn sqrt_approx(x: i32) -> i32 {
    if x <= 0 {
        return 0;
    }
    let (lz, frac) = clz_frac(x);
    let mut y = if lz & 1 != 0 { 32768 } else { 46214 };
    y >>= lz >> 1;
    smlawb(y, y, smulbb(213, frac))
}

/// Approximation of `(a << q) / b`.
pub fn div32_varq(a32: i32, b32: i32, qres: i32) -> i32 {
    let a_headrm = clz32(a32.wrapping_abs()) - 1;
    let mut a32_nrm = a32.wrapping_shl(a_headrm as u32);
    let b_headrm = clz32(b32.wrapping_abs()) - 1;
    let b32_nrm = b32.wrapping_shl(b_headrm as u32);
    let b32_inv = (i32::MAX >> 2) / (b32_nrm >> 16);
    let mut result = smulwb(a32_nrm, b32_inv);
    a32_nrm = a32_nrm.wrapping_sub(smmul(b32_nrm, result).wrapping_shl(3));
    result = smlawb(result, a32_nrm, b32_inv);
    let lshift = 29 + a_headrm - b_headrm - qres;
    if lshift < 0 {
        lshift_sat32(result, (-lshift) as u32)
    } else if lshift < 32 {
        result >> lshift
    } else {
        0
    }
}

/// Approximation of `(1 << q) / b`.
pub fn inverse32_varq(b32: i32, qres: i32) -> i32 {
    let b_headrm = clz32(b32.wrapping_abs()) - 1;
    let b32_nrm = b32.wrapping_shl(b_headrm as u32);
    let b32_inv = (i32::MAX >> 2) / (b32_nrm >> 16);
    let mut result = b32_inv << 16;
    let err_q32 = ((1i32 << 29) - smulwb(b32_nrm, b32_inv)).wrapping_shl(3);
    result = smlaww(result, err_q32, b32_inv);
    let lshift = 61 - b_headrm - qres;
    if lshift <= 0 {
        lshift_sat32(result, (-lshift) as u32)
    } else if lshift < 32 {
        result >> lshift
    } else {
        0
    }
}

/// `2^(x/128)` approximation.
pub fn log2lin(in_log_q7: i32) -> i32 {
    if in_log_q7 < 0 {
        return 0;
    } else if in_log_q7 >= 3967 {
        return i32::MAX;
    }
    let mut out = 1i32 << (in_log_q7 >> 7);
    let frac = in_log_q7 & 0x7f;
    let poly = smlawb(frac, smulbb(frac, 128 - frac), -174);
    if in_log_q7 < 2048 {
        out = out.wrapping_add(out.wrapping_mul(poly) >> 7);
    } else {
        out = out.wrapping_add((out >> 7).wrapping_mul(poly));
    }
    out
}

/// Energy of `x` with a right shift that keeps it within 32 bits.
pub fn sum_sqr_shift(x: &[i16]) -> (i32, i32) {
    let len = x.len() as i32;
    let pass = |shft: i32, init: i32| -> i32 {
        let mut nrg = init;
        let mut i = 0;
        while i + 1 < x.len() {
            let t = (smulbb(i32::from(x[i]), i32::from(x[i])) as u32)
                .wrapping_add(smulbb(i32::from(x[i + 1]), i32::from(x[i + 1])) as u32);
            nrg = (nrg as u32).wrapping_add(t >> shft) as i32;
            i += 2;
        }
        if i < x.len() {
            let t = smulbb(i32::from(x[i]), i32::from(x[i])) as u32;
            nrg = (nrg as u32).wrapping_add(t >> shft) as i32;
        }
        nrg
    };
    let shft = 31 - clz32(len);
    let nrg = pass(shft, len);
    let shft = 0.max(shft + 3 - clz32(nrg));
    (pass(shft, 0), shft)
}
