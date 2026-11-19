//! NLSF dequantisation and conversion to LPC coefficients (RFC 6716
//! sections 4.2.7.5.3 - 4.2.7.5.8), in the reference fixed-point.

use super::macros::*;
use super::tables::{NlsfCodebook, LSF_COS_TAB_Q12};

pub const MAX_LPC_ORDER: usize = 16;
const NLSF_QUANT_MAX_AMPLITUDE: i32 = 4;

/// Entropy-table offsets and predictor weights for a stage-1 vector.
pub fn nlsf_unpack(ec_ix: &mut [i16], pred_q8: &mut [u8], cb: &NlsfCodebook, cb1_index: usize) {
    let order = cb.order;
    let sel = &cb.ec_sel[cb1_index * order / 2..];
    for i in (0..order).step_by(2) {
        let entry = sel[i / 2];
        ec_ix[i] = (((entry >> 1) & 7) as i32 * (2 * NLSF_QUANT_MAX_AMPLITUDE + 1)) as i16;
        pred_q8[i] = cb.pred_q8[i + usize::from(entry & 1) * (order - 1)];
        ec_ix[i + 1] = (((entry >> 5) & 7) as i32 * (2 * NLSF_QUANT_MAX_AMPLITUDE + 1)) as i16;
        pred_q8[i + 1] = cb.pred_q8[i + usize::from((entry >> 4) & 1) * (order - 1) + 1];
    }
}

/// Decode an NLSF vector (Q15) from its indices.
pub fn nlsf_decode(nlsf_q15: &mut [i16], indices: &[i8], cb: &NlsfCodebook) {
    let order = cb.order;
    let mut pred_q8 = [0u8; MAX_LPC_ORDER];
    let mut ec_ix = [0i16; MAX_LPC_ORDER];
    let cb1 = indices[0] as usize;
    nlsf_unpack(&mut ec_ix, &mut pred_q8, cb, cb1);
    let mut res_q10 = [0i16; MAX_LPC_ORDER];
    let mut out_q10 = 0i32;
    for i in (0..order).rev() {
        let pred_q10 = smulbb(out_q10, i32::from(pred_q8[i])) >> 8;
        out_q10 = i32::from(indices[i + 1]) << 10;
        let adj = 102; // SILK_FIX_CONST(0.1, 10)
        if out_q10 > 0 {
            out_q10 -= adj;
        } else if out_q10 < 0 {
            out_q10 += adj;
        }
        out_q10 = smlawb(pred_q10, out_q10, cb.quant_step_size_q16);
        res_q10[i] = out_q10 as i16;
    }
    let el = &cb.cb1_q8[cb1 * order..];
    let wt = &cb.cb1_wght_q9[cb1 * order..];
    for i in 0..order {
        let v = ((i32::from(res_q10[i]) << 14) / i32::from(wt[i])) + (i32::from(el[i]) << 7);
        nlsf_q15[i] = v.clamp(0, 32767) as i16;
    }
    nlsf_stabilize(&mut nlsf_q15[..order], cb.delta_min_q15);
}

/// Enforce minimum spacing between NLSFs.
pub fn nlsf_stabilize(nlsf: &mut [i16], delta_min: &[i16]) {
    let l = nlsf.len();
    const MAX_LOOPS: usize = 20;
    for _ in 0..MAX_LOOPS {
        let mut min_diff = i32::from(nlsf[0]) - i32::from(delta_min[0]);
        let mut idx = 0;
        for i in 1..l {
            let diff = i32::from(nlsf[i]) - (i32::from(nlsf[i - 1]) + i32::from(delta_min[i]));
            if diff < min_diff {
                min_diff = diff;
                idx = i;
            }
        }
        let diff = (1 << 15) - (i32::from(nlsf[l - 1]) + i32::from(delta_min[l]));
        if diff < min_diff {
            min_diff = diff;
            idx = l;
        }
        if min_diff >= 0 {
            return;
        }
        if idx == 0 {
            nlsf[0] = delta_min[0];
        } else if idx == l {
            nlsf[l - 1] = ((1 << 15) - i32::from(delta_min[l])) as i16;
        } else {
            let mut min_center = 0i32;
            for k in 0..idx {
                min_center += i32::from(delta_min[k]);
            }
            min_center += i32::from(delta_min[idx]) >> 1;
            let mut max_center = 1i32 << 15;
            for k in (idx + 1..=l).rev() {
                max_center -= i32::from(delta_min[k]);
            }
            max_center -= i32::from(delta_min[idx]) >> 1;
            let center = limit(
                rshift_round(i32::from(nlsf[idx - 1]) + i32::from(nlsf[idx]), 1),
                min_center,
                max_center,
            ) as i16;
            nlsf[idx - 1] = (i32::from(center) - (i32::from(delta_min[idx]) >> 1)) as i16;
            nlsf[idx] = (i32::from(nlsf[idx - 1]) + i32::from(delta_min[idx])) as i16;
        }
    }
    // Fallback: sort, then clamp from both ends.
    for i in 1..l {
        let v = nlsf[i];
        let mut j = i;
        while j > 0 && v < nlsf[j - 1] {
            nlsf[j] = nlsf[j - 1];
            j -= 1;
        }
        nlsf[j] = v;
    }
    nlsf[0] = nlsf[0].max(delta_min[0]);
    for i in 1..l {
        let floor = sat16(i32::from(nlsf[i - 1]) + i32::from(delta_min[i])) as i16;
        nlsf[i] = nlsf[i].max(floor);
    }
    nlsf[l - 1] = (i32::from(nlsf[l - 1]).min((1 << 15) - i32::from(delta_min[l]))) as i16;
    for i in (0..l - 1).rev() {
        nlsf[i] =
            (i32::from(nlsf[i]).min(i32::from(nlsf[i + 1]) - i32::from(delta_min[i + 1]))) as i16;
    }
}

/// Chirp an int16 AR filter.
pub fn bwexpander(ar: &mut [i16], mut chirp_q16: i32) {
    let d = ar.len();
    let chirp_minus_one = chirp_q16 - 65536;
    for v in ar.iter_mut().take(d - 1) {
        *v = rshift_round(chirp_q16.wrapping_mul(i32::from(*v)), 16) as i16;
        chirp_q16 += rshift_round(chirp_q16.wrapping_mul(chirp_minus_one), 16);
    }
    ar[d - 1] = rshift_round(chirp_q16.wrapping_mul(i32::from(ar[d - 1])), 16) as i16;
}

/// Chirp an int32 AR filter.
pub fn bwexpander_32(ar: &mut [i32], mut chirp_q16: i32) {
    let d = ar.len();
    let chirp_minus_one = chirp_q16 - 65536;
    for v in ar.iter_mut().take(d - 1) {
        *v = smulww(chirp_q16, *v);
        chirp_q16 += rshift_round(chirp_q16.wrapping_mul(chirp_minus_one), 16);
    }
    ar[d - 1] = smulww(chirp_q16, ar[d - 1]);
}

/// Convert 32-bit coefficients to int16 without wrap-around.
fn lpc_fit(a_qout: &mut [i16], a_qin: &mut [i32], qout: u32, qin: u32) {
    let d = a_qin.len();
    let mut i = 0;
    while i < 10 {
        let mut maxabs = 0i32;
        let mut idx = 0usize;
        for (k, &v) in a_qin.iter().enumerate() {
            let a = v.wrapping_abs();
            if a > maxabs {
                maxabs = a;
                idx = k;
            }
        }
        maxabs = rshift_round(maxabs, qin - qout);
        if maxabs > i32::from(i16::MAX) {
            maxabs = maxabs.min(163_838);
            let chirp = 65470 // SILK_FIX_CONST(0.999, 16)
                - ((maxabs - i32::from(i16::MAX)) << 14) / ((maxabs * (idx as i32 + 1)) >> 2);
            bwexpander_32(a_qin, chirp);
        } else {
            break;
        }
        i += 1;
    }
    if i == 10 {
        for k in 0..d {
            a_qout[k] = sat16(rshift_round(a_qin[k], qin - qout)) as i16;
            a_qin[k] = i32::from(a_qout[k]) << (qin - qout);
        }
    } else {
        for k in 0..d {
            a_qout[k] = rshift_round(a_qin[k], qin - qout) as i16;
        }
    }
}

const QA_INV: u32 = 24;
const A_LIMIT: i32 = 16_773_022; // SILK_FIX_CONST(0.99975, 24)

fn inverse_pred_gain_qa(a: &mut [i32]) -> i32 {
    let order = a.len();
    let mut inv_gain_q30 = 1i32 << 30;
    let min_inv_gain = 107_374; // SILK_FIX_CONST(1.0 / 1e4, 30)
    for k in (1..order).rev() {
        if a[k] > A_LIMIT || a[k] < -A_LIMIT {
            return 0;
        }
        let rc_q31 = -(a[k] << (31 - QA_INV));
        let rc_mult1_q30 = (1i32 << 30) - smmul(rc_q31, rc_q31);
        inv_gain_q30 = smmul(inv_gain_q30, rc_mult1_q30) << 2;
        if inv_gain_q30 < min_inv_gain {
            return 0;
        }
        let mult2q = 32 - clz32(rc_mult1_q30.abs());
        let rc_mult2 = inverse32_varq(rc_mult1_q30, mult2q + 30);
        for n in 0..(k + 1) >> 1 {
            let tmp1 = a[n];
            let tmp2 = a[k - n - 1];
            let mul_frac = |x: i32| rshift_round64(i64::from(x) * i64::from(rc_q31), 31) as i32;
            let t = rshift_round64(
                i64::from(sub_sat32(tmp1, mul_frac(tmp2))) * i64::from(rc_mult2),
                mult2q as u32,
            );
            if t > i64::from(i32::MAX) || t < i64::from(i32::MIN) {
                return 0;
            }
            a[n] = t as i32;
            let t = rshift_round64(
                i64::from(sub_sat32(tmp2, mul_frac(tmp1))) * i64::from(rc_mult2),
                mult2q as u32,
            );
            if t > i64::from(i32::MAX) || t < i64::from(i32::MIN) {
                return 0;
            }
            a[k - n - 1] = t as i32;
        }
    }
    if a[0] > A_LIMIT || a[0] < -A_LIMIT {
        return 0;
    }
    let rc_q31 = -(a[0] << (31 - QA_INV));
    let rc_mult1_q30 = (1i32 << 30) - smmul(rc_q31, rc_q31);
    inv_gain_q30 = smmul(inv_gain_q30, rc_mult1_q30) << 2;
    if inv_gain_q30 < min_inv_gain {
        return 0;
    }
    inv_gain_q30
}

/// Inverse prediction gain (Q30) of a Q12 filter, 0 if unstable.
pub fn lpc_inverse_pred_gain(a_q12: &[i16]) -> i32 {
    let mut atmp = [0i32; MAX_LPC_ORDER];
    let mut dc_resp = 0i32;
    for (k, &v) in a_q12.iter().enumerate() {
        dc_resp += i32::from(v);
        atmp[k] = i32::from(v) << (QA_INV - 12);
    }
    if dc_resp >= 4096 {
        return 0;
    }
    inverse_pred_gain_qa(&mut atmp[..a_q12.len()])
}

const QA: u32 = 16;

fn find_poly(out: &mut [i32], c_lsf: &[i32], stride_off: usize, dd: usize) {
    out[0] = 1 << QA;
    out[1] = -c_lsf[stride_off];
    for k in 1..dd {
        let ftmp = c_lsf[2 * k + stride_off];
        out[k + 1] =
            (out[k - 1] << 1) - rshift_round64(i64::from(ftmp) * i64::from(out[k]), QA) as i32;
        for n in (2..=k).rev() {
            out[n] +=
                out[n - 2] - rshift_round64(i64::from(ftmp) * i64::from(out[n - 1]), QA) as i32;
        }
        out[1] -= ftmp;
    }
}

/// NLSF (Q15) to monic whitening filter coefficients (Q12).
pub fn nlsf2a(a_q12: &mut [i16], nlsf: &[i16]) {
    const ORDERING16: [usize; 16] = [0, 15, 8, 7, 4, 11, 12, 3, 2, 13, 10, 5, 6, 9, 14, 1];
    const ORDERING10: [usize; 10] = [0, 9, 6, 3, 4, 5, 8, 1, 2, 7];
    let d = nlsf.len();
    let ordering: &[usize] = if d == 16 { &ORDERING16 } else { &ORDERING10 };
    let mut cos_lsf = [0i32; MAX_LPC_ORDER];
    for k in 0..d {
        let n = i32::from(nlsf[k]);
        let f_int = n >> (15 - 7);
        let f_frac = n - (f_int << (15 - 7));
        let cos_val = i32::from(LSF_COS_TAB_Q12[f_int as usize]);
        let delta = i32::from(LSF_COS_TAB_Q12[f_int as usize + 1]) - cos_val;
        cos_lsf[ordering[k]] = rshift_round((cos_val << 8) + delta * f_frac, 20 - QA);
    }
    let dd = d >> 1;
    let mut p = [0i32; MAX_LPC_ORDER / 2 + 1];
    let mut q = [0i32; MAX_LPC_ORDER / 2 + 1];
    find_poly(&mut p, &cos_lsf, 0, dd);
    find_poly(&mut q, &cos_lsf, 1, dd);
    let mut a32 = [0i32; MAX_LPC_ORDER];
    for k in 0..dd {
        let ptmp = p[k + 1] + p[k];
        let qtmp = q[k + 1] - q[k];
        a32[k] = -qtmp - ptmp;
        a32[d - k - 1] = qtmp - ptmp;
    }
    lpc_fit(&mut a_q12[..d], &mut a32[..d], 12, QA + 1);
    let mut i = 0;
    while lpc_inverse_pred_gain(&a_q12[..d]) == 0 && i < 16 {
        bwexpander_32(&mut a32[..d], 65536 - (2 << i));
        for k in 0..d {
            a_q12[k] = rshift_round(a32[k], QA + 1 - 12) as i16;
        }
        i += 1;
    }
}

/// MA filter `out[i] = in[i] - sum B[j] in[i-1-j]` (Q12), first `d` zero.
pub fn lpc_analysis_filter(out: &mut [i16], input: &[i16], b: &[i16], len: usize) {
    let d = b.len();
    for ix in d..len {
        let mut acc = smulbb(i32::from(input[ix - 1]), i32::from(b[0]));
        for j in 1..d {
            acc = acc.wrapping_add(smulbb(i32::from(input[ix - 1 - j]), i32::from(b[j])));
        }
        let v = (i32::from(input[ix]) << 12).wrapping_sub(acc);
        out[ix] = sat16(rshift_round(v, 12)) as i16;
    }
    out[..d].fill(0);
}
