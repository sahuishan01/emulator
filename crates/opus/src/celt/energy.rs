//! Band energy decoding (RFC 6716 section 4.3.2).

use super::tables::*;
use crate::range::RangeDecoder;

/// Coarse energy: Laplace-coded residuals of a 2-D (time and frequency)
/// prediction. `old` holds both channels (`c * NB_EBANDS + band`).
pub fn unquant_coarse(
    start: usize,
    end: usize,
    old: &mut [f32; 2 * NB_EBANDS],
    intra: bool,
    dec: &mut RangeDecoder,
    c: usize,
    lm: usize,
) {
    let model = &E_PROB_MODEL[lm * 84 + usize::from(intra) * 42..][..42];
    let (coef, beta) = if intra {
        (0.0, BETA_INTRA)
    } else {
        (PRED_COEF[lm], BETA_COEF[lm])
    };
    let mut prev = [0f32; 2];
    let budget = dec.storage() as i32 * 8;
    for i in start..end {
        for ch in 0..c {
            let tell = dec.tell();
            let qi = if budget - tell >= 15 {
                let pi = 2 * i.min(20);
                dec.laplace(u32::from(model[pi]) << 7, u32::from(model[pi + 1]) << 6)
            } else if budget - tell >= 2 {
                let qi = dec.icdf(&SMALL_ENERGY_ICDF, 2) as i32;
                (qi >> 1) ^ -(qi & 1)
            } else if budget - tell >= 1 {
                -i32::from(dec.bit_logp(1))
            } else {
                -1
            };
            let q = qi as f32;
            let e = &mut old[i + ch * NB_EBANDS];
            *e = e.max(-9.0);
            let tmp = coef * *e + prev[ch] + q;
            *e = tmp;
            prev[ch] = prev[ch] + q - beta * q;
        }
    }
}

pub fn unquant_fine(
    start: usize,
    end: usize,
    old: &mut [f32; 2 * NB_EBANDS],
    fine_quant: &[i32; NB_EBANDS],
    dec: &mut RangeDecoder,
    c: usize,
) {
    for i in start..end {
        let fq = fine_quant[i];
        if fq <= 0 {
            continue;
        }
        for ch in 0..c {
            let q2 = dec.bits(fq as u32);
            let offset = (q2 as f32 + 0.5) * (1 << (14 - fq)) as f32 * (1.0 / 16384.0) - 0.5;
            old[i + ch * NB_EBANDS] += offset;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn unquant_finalise(
    start: usize,
    end: usize,
    old: &mut [f32; 2 * NB_EBANDS],
    fine_quant: &[i32; NB_EBANDS],
    fine_priority: &[i32; NB_EBANDS],
    mut bits_left: i32,
    dec: &mut RangeDecoder,
    c: usize,
) {
    for prio in 0..2 {
        let mut i = start;
        while i < end && bits_left >= c as i32 {
            if fine_quant[i] >= MAX_FINE_BITS || fine_priority[i] != prio {
                i += 1;
                continue;
            }
            for ch in 0..c {
                let q2 = dec.bits(1);
                let offset =
                    (q2 as f32 - 0.5) * (1 << (14 - fine_quant[i] - 1)) as f32 * (1.0 / 16384.0);
                old[i + ch * NB_EBANDS] += offset;
                bits_left -= 1;
            }
            i += 1;
        }
    }
}
