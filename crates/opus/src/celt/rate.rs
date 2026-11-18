//! CELT bit allocation (RFC 6716 section 4.3.3).

use super::tables::*;
use crate::range::{RangeDecoder, BITRES};

const ALLOC_STEPS: i32 = 6;
const MAX_PSEUDO: usize = 40;

/// Pulse count for a pseudo-pulse index.
#[inline]
pub fn get_pulses(i: i32) -> i32 {
    if i < 8 {
        i
    } else {
        (8 + (i & 7)) << ((i >> 3) - 1)
    }
}

#[inline]
fn cache_for(band: usize, lm: i32) -> &'static [u8] {
    let idx = CACHE_INDEX50[(lm + 1) as usize * NB_EBANDS + band];
    &CACHE_BITS50[idx as usize..]
}

/// Largest cached bit cost for a band split decision.
#[inline]
pub fn cache_max(band: usize, lm: i32) -> i32 {
    let c = cache_for(band, lm);
    i32::from(c[usize::from(c[0])])
}

pub fn bits2pulses(band: usize, lm: i32, bits: i32) -> i32 {
    let cache = cache_for(band, lm);
    let mut lo = 0i32;
    let mut hi = i32::from(cache[0]);
    let bits = bits - 1;
    for _ in 0..6 {
        let mid = (lo + hi + 1) >> 1;
        if i32::from(cache[mid as usize]) >= bits {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let lo_bits = if lo == 0 { -1 } else { i32::from(cache[lo as usize]) };
    if bits - lo_bits <= i32::from(cache[hi as usize]) - bits {
        lo
    } else {
        hi
    }
}

pub fn pulses2bits(band: usize, lm: i32, pulses: i32) -> i32 {
    if pulses == 0 {
        0
    } else {
        i32::from(cache_for(band, lm)[pulses as usize]) + 1
    }
}

/// Per-band maximum useful allocation.
pub fn init_caps(cap: &mut [i32; NB_EBANDS], lm: usize, c: usize) {
    for (i, cap) in cap.iter_mut().enumerate() {
        let n = i32::from(EBANDS[i + 1] - EBANDS[i]) << lm;
        *cap = (i32::from(CACHE_CAPS50[NB_EBANDS * (2 * lm + c - 1) + i]) + 64) * c as i32 * n
            >> 2;
    }
}

/// Output of the allocation.
pub struct Allocation {
    pub coded_bands: usize,
    pub intensity: usize,
    pub dual_stereo: bool,
    pub balance: i32,
    pub pulses: [i32; NB_EBANDS],
    pub fine_quant: [i32; NB_EBANDS],
    pub fine_priority: [i32; NB_EBANDS],
}

#[inline]
fn band_width(j: usize) -> i32 {
    i32::from(EBANDS[j + 1] - EBANDS[j])
}

#[allow(clippy::too_many_arguments)]
pub fn compute_allocation(
    start: usize,
    end: usize,
    offsets: &[i32; NB_EBANDS],
    cap: &[i32; NB_EBANDS],
    alloc_trim: i32,
    total: i32,
    c: usize,
    lm: usize,
    dec: &mut RangeDecoder,
) -> Allocation {
    let cc = c as i32;
    let lmi = lm as i32;
    let mut total = total.max(0);
    let mut skip_start = start;
    let skip_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
    total -= skip_rsv;
    let mut intensity_rsv = 0;
    let mut dual_stereo_rsv = 0;
    if c == 2 {
        intensity_rsv = i32::from(LOG2_FRAC_TABLE[end - start]);
        if intensity_rsv > total {
            intensity_rsv = 0;
        } else {
            total -= intensity_rsv;
            dual_stereo_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
            total -= dual_stereo_rsv;
        }
    }
    let mut bits1 = [0i32; NB_EBANDS];
    let mut bits2 = [0i32; NB_EBANDS];
    let mut thresh = [0i32; NB_EBANDS];
    let mut trim_offset = [0i32; NB_EBANDS];
    for j in start..end {
        let w = band_width(j);
        thresh[j] = (cc << BITRES).max((3 * w << lm << BITRES) >> 4);
        trim_offset[j] =
            cc * w * (alloc_trim - 5 - lmi) * (end - j - 1) as i32 * (1 << (lm as u32 + BITRES)) >> 6;
        if w << lm == 1 {
            trim_offset[j] -= cc << BITRES;
        }
    }
    let alloc_vec = |row: usize, j: usize| -> i32 {
        cc * band_width(j) * i32::from(BAND_ALLOCATION[row * NB_EBANDS + j]) << lm >> 2
    };
    let mut lo = 1i32;
    let mut hi = NB_ALLOC_VECTORS as i32 - 1;
    loop {
        let mut done = false;
        let mut psum = 0;
        let mid = (lo + hi) >> 1;
        for j in (start..end).rev() {
            let mut bitsj = alloc_vec(mid as usize, j);
            if bitsj > 0 {
                bitsj = (bitsj + trim_offset[j]).max(0);
            }
            bitsj += offsets[j];
            if bitsj >= thresh[j] || done {
                done = true;
                psum += bitsj.min(cap[j]);
            } else if bitsj >= cc << BITRES {
                psum += cc << BITRES;
            }
        }
        if psum > total {
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
        if lo > hi {
            break;
        }
    }
    hi = lo;
    lo -= 1;
    for j in start..end {
        let mut b1 = alloc_vec(lo as usize, j);
        let mut b2 = if hi as usize >= NB_ALLOC_VECTORS {
            cap[j]
        } else {
            alloc_vec(hi as usize, j)
        };
        if b1 > 0 {
            b1 = (b1 + trim_offset[j]).max(0);
        }
        if b2 > 0 {
            b2 = (b2 + trim_offset[j]).max(0);
        }
        if lo > 0 {
            b1 += offsets[j];
        }
        b2 += offsets[j];
        if offsets[j] > 0 {
            skip_start = j;
        }
        bits1[j] = b1;
        bits2[j] = (b2 - b1).max(0);
    }
    interp_bits2pulses(
        start,
        end,
        skip_start,
        &bits1,
        &bits2,
        &thresh,
        cap,
        total,
        skip_rsv,
        intensity_rsv,
        dual_stereo_rsv,
        c,
        lm,
        dec,
    )
}

#[allow(clippy::too_many_arguments)]
fn interp_bits2pulses(
    start: usize,
    end: usize,
    skip_start: usize,
    bits1: &[i32; NB_EBANDS],
    bits2: &[i32; NB_EBANDS],
    thresh: &[i32; NB_EBANDS],
    cap: &[i32; NB_EBANDS],
    mut total: i32,
    skip_rsv: i32,
    mut intensity_rsv: i32,
    mut dual_stereo_rsv: i32,
    c: usize,
    lm: usize,
    dec: &mut RangeDecoder,
) -> Allocation {
    let cc = c as i32;
    let alloc_floor = cc << BITRES;
    let stereo = (c > 1) as i32;
    let log_m = (lm as i32) << BITRES;
    let mut lo = 0;
    let mut hi = 1 << ALLOC_STEPS;
    for _ in 0..ALLOC_STEPS {
        let mid = (lo + hi) >> 1;
        let mut psum = 0;
        let mut done = false;
        for j in (start..end).rev() {
            let tmp = bits1[j] + (mid * bits2[j] >> ALLOC_STEPS);
            if tmp >= thresh[j] || done {
                done = true;
                psum += tmp.min(cap[j]);
            } else if tmp >= alloc_floor {
                psum += alloc_floor;
            }
        }
        if psum > total {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let mut bits = [0i32; NB_EBANDS];
    let mut psum = 0;
    let mut done = false;
    for j in (start..end).rev() {
        let mut tmp = bits1[j] + (lo * bits2[j] >> ALLOC_STEPS);
        if tmp < thresh[j] && !done {
            tmp = if tmp >= alloc_floor { alloc_floor } else { 0 };
        } else {
            done = true;
        }
        tmp = tmp.min(cap[j]);
        bits[j] = tmp;
        psum += tmp;
    }
    let ebands = |i: usize| i32::from(EBANDS[i]);
    let mut coded_bands = end;
    loop {
        let j = coded_bands - 1;
        if j <= skip_start {
            total += skip_rsv;
            break;
        }
        let mut left = total - psum;
        let percoeff = left / (ebands(coded_bands) - ebands(start));
        left -= (ebands(coded_bands) - ebands(start)) * percoeff;
        let rem = (left - (ebands(j) - ebands(start))).max(0);
        let band_width = ebands(coded_bands) - ebands(j);
        let mut band_bits = bits[j] + percoeff * band_width + rem;
        if band_bits >= thresh[j].max(alloc_floor + (1 << BITRES)) {
            if dec.bit_logp(1) {
                break;
            }
            psum += 1 << BITRES;
            band_bits -= 1 << BITRES;
        }
        psum -= bits[j] + intensity_rsv;
        if intensity_rsv > 0 {
            intensity_rsv = i32::from(LOG2_FRAC_TABLE[j - start]);
        }
        psum += intensity_rsv;
        if band_bits >= alloc_floor {
            psum += alloc_floor;
            bits[j] = alloc_floor;
        } else {
            bits[j] = 0;
        }
        coded_bands -= 1;
    }
    let mut intensity = 0usize;
    if intensity_rsv > 0 {
        intensity = start + dec.uint((coded_bands + 1 - start) as u32) as usize;
    }
    if intensity <= start {
        total += dual_stereo_rsv;
        dual_stereo_rsv = 0;
    }
    let dual_stereo = dual_stereo_rsv > 0 && dec.bit_logp(1);

    let mut left = total - psum;
    let percoeff = left / (ebands(coded_bands) - ebands(start));
    left -= (ebands(coded_bands) - ebands(start)) * percoeff;
    for j in start..coded_bands {
        bits[j] += percoeff * band_width(j);
    }
    for j in start..coded_bands {
        let tmp = left.min(band_width(j));
        bits[j] += tmp;
        left -= tmp;
    }

    let mut ebits = [0i32; NB_EBANDS];
    let mut fine_priority = [0i32; NB_EBANDS];
    let mut balance = 0;
    let mut j = start;
    while j < coded_bands {
        let n0 = band_width(j);
        let n = n0 << lm;
        let bit = bits[j] + balance;
        let mut excess;
        if n > 1 {
            excess = (bit - cap[j]).max(0);
            bits[j] = bit - excess;
            let den = cc * n
                + i32::from(c == 2 && n > 2 && !dual_stereo && j < intensity);
            let nclogn = den * (i32::from(LOGN400[j]) + log_m);
            let mut offset = (nclogn >> 1) - den * FINE_OFFSET;
            if n == 2 {
                offset += den << BITRES >> 2;
            }
            if bits[j] + offset < den * 2 << BITRES {
                offset += nclogn >> 2;
            } else if bits[j] + offset < den * 3 << BITRES {
                offset += nclogn >> 3;
            }
            ebits[j] = (bits[j] + offset + (den << (BITRES - 1))).max(0);
            ebits[j] = (ebits[j] / den) >> BITRES;
            if cc * ebits[j] > bits[j] >> BITRES {
                ebits[j] = bits[j] >> stereo >> BITRES;
            }
            ebits[j] = ebits[j].min(MAX_FINE_BITS);
            fine_priority[j] = i32::from(ebits[j] * (den << BITRES) >= bits[j] + offset);
            bits[j] -= cc * ebits[j] << BITRES;
        } else {
            excess = (bit - (cc << BITRES)).max(0);
            bits[j] = bit - excess;
            ebits[j] = 0;
            fine_priority[j] = 1;
        }
        if excess > 0 {
            let extra_fine = (excess >> (stereo + BITRES as i32)).min(MAX_FINE_BITS - ebits[j]);
            ebits[j] += extra_fine;
            let extra_bits = extra_fine * cc << BITRES;
            fine_priority[j] = i32::from(extra_bits >= excess - balance);
            excess -= extra_bits;
        }
        balance = excess;
        j += 1;
    }
    while j < end {
        ebits[j] = bits[j] >> stereo >> BITRES;
        bits[j] = 0;
        fine_priority[j] = i32::from(ebits[j] < 1);
        j += 1;
    }
    let _ = MAX_PSEUDO;
    Allocation {
        coded_bands,
        intensity,
        dual_stereo,
        balance,
        pulses: bits,
        fine_quant: ebits,
        fine_priority,
    }
}
