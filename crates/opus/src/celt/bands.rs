//! Band shape decoding (RFC 6716 section 4.3.4): recursive band splitting,
//! stereo, spectral folding, TF resolution changes and anti-collapse.

use super::pvq::{alg_unquant, renormalise_vector};
use super::rate::{bits2pulses, cache_max, get_pulses, pulses2bits};
use super::tables::*;
use crate::range::{ilog, RangeDecoder, BITRES};

/// Largest band length in samples (band 20 at LM 3).
pub const MAX_BAND: usize = 176;

#[inline]
pub fn lcg_rand(seed: u32) -> u32 {
    seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

#[inline]
fn frac_mul16(a: i32, b: i32) -> i32 {
    (16384 + a * b) >> 15
}

/// Bit-exact cosine approximation used for the split allocation.
fn bitexact_cos(x: i32) -> i32 {
    let tmp = (4096 + x * x) >> 13;
    let x2 = tmp;
    let x2 = (32767 - x2) + frac_mul16(x2, -7651 + frac_mul16(x2, 8277 + frac_mul16(-626, x2)));
    1 + x2
}

fn bitexact_log2tan(isin: i32, icos: i32) -> i32 {
    let lc = ilog(icos as u32) as i32;
    let ls = ilog(isin as u32) as i32;
    let icos = icos << (15 - lc);
    let isin = isin << (15 - ls);
    (ls - lc) * (1 << 11) + frac_mul16(isin, frac_mul16(isin, -2597) + 7932)
        - frac_mul16(icos, frac_mul16(icos, -2597) + 7932)
}

fn isqrt32(v: u32) -> u32 {
    // Bitwise integer square root (floor).
    let mut g = 0u32;
    let mut bshift = (ilog(v) as i32 - 1) >> 1;
    let mut b = 1u32 << bshift;
    let mut val = v;
    loop {
        let t = ((g << 1) + b) << bshift;
        if t <= val {
            g += b;
            val -= t;
        }
        b >>= 1;
        bshift -= 1;
        if bshift < 0 {
            break;
        }
    }
    g
}

fn haar1(x: &mut [f32], n0: usize, stride: usize) {
    let n0 = n0 >> 1;
    let s = std::f32::consts::FRAC_1_SQRT_2;
    for i in 0..stride {
        for j in 0..n0 {
            let a = x[stride * 2 * j + i];
            let b = x[stride * (2 * j + 1) + i];
            let t1 = s * a;
            let t2 = s * b;
            x[stride * 2 * j + i] = t1 + t2;
            x[stride * (2 * j + 1) + i] = t1 - t2;
        }
    }
}

fn deinterleave_hadamard(
    tmp: &mut [f32; MAX_BAND],
    x: &mut [f32],
    n0: usize,
    stride: usize,
    hadamard: bool,
) {
    let n = n0 * stride;
    if hadamard {
        let ordery = &ORDERY_TABLE[stride - 2..];
        for i in 0..stride {
            for j in 0..n0 {
                tmp[ordery[i] * n0 + j] = x[j * stride + i];
            }
        }
    } else {
        for i in 0..stride {
            for j in 0..n0 {
                tmp[i * n0 + j] = x[j * stride + i];
            }
        }
    }
    x[..n].copy_from_slice(&tmp[..n]);
}

fn interleave_hadamard(
    tmp: &mut [f32; MAX_BAND],
    x: &mut [f32],
    n0: usize,
    stride: usize,
    hadamard: bool,
) {
    let n = n0 * stride;
    if hadamard {
        let ordery = &ORDERY_TABLE[stride - 2..];
        for i in 0..stride {
            for j in 0..n0 {
                tmp[j * stride + i] = x[ordery[i] * n0 + j];
            }
        }
    } else {
        for i in 0..stride {
            for j in 0..n0 {
                tmp[j * stride + i] = x[i * n0 + j];
            }
        }
    }
    x[..n].copy_from_slice(&tmp[..n]);
}

fn compute_qn(n: i32, b: i32, offset: i32, pulse_cap: i32, stereo: bool) -> i32 {
    const EXP2_TABLE8: [i32; 8] = [16384, 17866, 19483, 21247, 23170, 25267, 27554, 30048];
    let mut n2 = 2 * n - 1;
    if stereo && n == 2 {
        n2 -= 1;
    }
    let mut qb = sdiv(b + n2 * offset, n2);
    qb = qb.min(b - pulse_cap - (4 << BITRES));
    qb = qb.min(8 << BITRES);
    if qb < (1 << BITRES >> 1) {
        1
    } else {
        let qn = EXP2_TABLE8[(qb & 7) as usize] >> (14 - (qb >> BITRES));
        (qn + 1) >> 1 << 1
    }
}

/// Signed division truncating toward zero (C semantics).
#[inline]
fn sdiv(a: i32, b: i32) -> i32 {
    a / b
}

struct Split {
    inv: bool,
    imid: i32,
    iside: i32,
    delta: i32,
    itheta: i32,
    qalloc: i32,
}

pub struct BandCtx<'a, 'b> {
    pub dec: &'a mut RangeDecoder<'b>,
    pub i: usize,
    pub intensity: usize,
    pub spread: usize,
    pub tf_change: i32,
    pub remaining_bits: i32,
    pub seed: u32,
    pub disable_inv: bool,
    pub avoid_split_noise: bool,
    iy: [i32; MAX_BAND],
    /// Permutation scratch for the Hadamard (de)interleave.
    tmp: [f32; MAX_BAND],
}

impl BandCtx<'_, '_> {
    #[allow(clippy::too_many_arguments)]
    fn compute_theta(
        &mut self,
        n: i32,
        b: &mut i32,
        bb: i32,
        b0: i32,
        lm: i32,
        stereo: bool,
        fill: &mut u32,
    ) -> Split {
        let i = self.i;
        let pulse_cap = i32::from(LOGN400[i]) + lm * (1 << BITRES);
        let offset = (pulse_cap >> 1)
            - if stereo && n == 2 {
                QTHETA_OFFSET_TWOPHASE
            } else {
                QTHETA_OFFSET
            };
        let mut qn = compute_qn(n, *b, offset, pulse_cap, stereo);
        if stereo && i >= self.intensity {
            qn = 1;
        }
        let tell = self.dec.tell_frac() as i32;
        let mut itheta = 0i32;
        let mut inv = false;
        if qn != 1 {
            if stereo && n > 2 {
                let p0 = 3;
                let x0 = qn / 2;
                let ft = p0 * (x0 + 1) + x0;
                let fs = self.dec.decode(ft as u32) as i32;
                let x = if fs < (x0 + 1) * p0 {
                    fs / p0
                } else {
                    x0 + 1 + (fs - (x0 + 1) * p0)
                };
                let (fl, fh) = if x <= x0 {
                    (p0 * x, p0 * (x + 1))
                } else {
                    ((x - 1 - x0) + (x0 + 1) * p0, (x - x0) + (x0 + 1) * p0)
                };
                self.dec.update(fl as u32, fh as u32, ft as u32);
                itheta = x;
            } else if b0 > 1 || stereo {
                itheta = self.dec.uint((qn + 1) as u32) as i32;
            } else {
                let ft = ((qn >> 1) + 1) * ((qn >> 1) + 1);
                let fm = self.dec.decode(ft as u32) as i32;
                let (fl, fs);
                if fm < ((qn >> 1) * ((qn >> 1) + 1) >> 1) {
                    itheta = (isqrt32(8 * fm as u32 + 1) as i32 - 1) >> 1;
                    fs = itheta + 1;
                    fl = itheta * (itheta + 1) >> 1;
                } else {
                    itheta = (2 * (qn + 1) - isqrt32(8 * (ft - fm - 1) as u32 + 1) as i32) >> 1;
                    fs = qn + 1 - itheta;
                    fl = ft - ((qn + 1 - itheta) * (qn + 2 - itheta) >> 1);
                }
                self.dec.update(fl as u32, (fl + fs) as u32, ft as u32);
            }
            itheta = itheta * 16384 / qn;
        } else if stereo {
            if *b > 2 << BITRES && self.remaining_bits > 2 << BITRES {
                inv = self.dec.bit_logp(2);
            }
            if self.disable_inv {
                inv = false;
            }
            itheta = 0;
        }
        let qalloc = self.dec.tell_frac() as i32 - tell;
        *b -= qalloc;
        let (imid, iside, delta);
        if itheta == 0 {
            imid = 32767;
            iside = 0;
            *fill &= (1u32 << bb) - 1;
            delta = -16384;
        } else if itheta == 16384 {
            imid = 0;
            iside = 32767;
            *fill &= ((1u32 << bb) - 1) << bb;
            delta = 16384;
        } else {
            imid = bitexact_cos(itheta);
            iside = bitexact_cos(16384 - itheta);
            delta = frac_mul16((n - 1) << 7, bitexact_log2tan(iside, imid));
        }
        Split {
            inv,
            imid,
            iside,
            delta,
            itheta,
            qalloc,
        }
    }

    fn quant_band_n1(
        &mut self,
        x: &mut [f32],
        y: Option<&mut [f32]>,
        lowband_out: Option<&mut [f32]>,
    ) -> u32 {
        let decode_sign = |ctx: &mut Self, v: &mut f32| {
            let mut sign = 0;
            if ctx.remaining_bits >= 1 << BITRES {
                sign = ctx.dec.bits(1);
                ctx.remaining_bits -= 1 << BITRES;
            }
            *v = if sign != 0 { -1.0 } else { 1.0 };
        };
        decode_sign(self, &mut x[0]);
        if let Some(y) = y {
            decode_sign(self, &mut y[0]);
        }
        if let Some(out) = lowband_out {
            out[0] = x[0];
        }
        1
    }

    #[allow(clippy::too_many_arguments)]
    fn quant_partition(
        &mut self,
        x: &mut [f32],
        n: usize,
        mut b: i32,
        mut bb: i32,
        lowband: Option<&[f32]>,
        mut lm: i32,
        gain: f32,
        mut fill: u32,
    ) -> u32 {
        let i = self.i;
        let b0 = bb;
        if lm != -1 && b > cache_max(i, lm) + 12 && n > 2 {
            let n = n >> 1;
            let (xs, ys) = x.split_at_mut(n);
            lm -= 1;
            if bb == 1 {
                fill = (fill & 1) | (fill << 1);
            }
            bb = (bb + 1) >> 1;
            let s = self.compute_theta(n as i32, &mut b, bb, b0, lm, false, &mut fill);
            let mid = (1.0 / 32768.0) * s.imid as f32;
            let side = (1.0 / 32768.0) * s.iside as f32;
            let mut delta = s.delta;
            let itheta = s.itheta;
            if b0 > 1 && (itheta & 0x3fff) != 0 {
                if itheta > 8192 {
                    delta -= delta >> (4 - lm);
                } else {
                    delta = (delta + ((n as i32) << BITRES >> (5 - lm))).min(0);
                }
            }
            let mut mbits = b.min((b - delta) / 2).max(0);
            let mut sbits = b - mbits;
            self.remaining_bits -= s.qalloc;
            let next_lowband2 = lowband.map(|l| &l[n..]);
            let mut rebalance = self.remaining_bits;
            let mut cm;
            if mbits >= sbits {
                cm = self.quant_partition(xs, n, mbits, bb, lowband, lm, gain * mid, fill);
                rebalance = mbits - (rebalance - self.remaining_bits);
                if rebalance > 3 << BITRES && itheta != 0 {
                    sbits += rebalance - (3 << BITRES);
                }
                cm |= self.quant_partition(
                    ys,
                    n,
                    sbits,
                    bb,
                    next_lowband2,
                    lm,
                    gain * side,
                    fill >> bb,
                ) << (b0 >> 1);
            } else {
                cm = self.quant_partition(
                    ys,
                    n,
                    sbits,
                    bb,
                    next_lowband2,
                    lm,
                    gain * side,
                    fill >> bb,
                ) << (b0 >> 1);
                rebalance = sbits - (rebalance - self.remaining_bits);
                if rebalance > 3 << BITRES && itheta != 16384 {
                    mbits += rebalance - (3 << BITRES);
                }
                cm |= self.quant_partition(xs, n, mbits, bb, lowband, lm, gain * mid, fill);
            }
            return cm;
        }
        let mut q = bits2pulses(i, lm, b);
        let mut curr_bits = pulses2bits(i, lm, q);
        self.remaining_bits -= curr_bits;
        while self.remaining_bits < 0 && q > 0 {
            self.remaining_bits += curr_bits;
            q -= 1;
            curr_bits = pulses2bits(i, lm, q);
            self.remaining_bits -= curr_bits;
        }
        if q != 0 {
            let k = get_pulses(q) as usize;
            return alg_unquant(
                x,
                n,
                k,
                self.spread,
                bb as usize,
                self.dec,
                gain,
                &mut self.iy,
            );
        }
        let cm_mask = ((1u64 << bb) - 1) as u32;
        fill &= cm_mask;
        if fill == 0 {
            x[..n].fill(0.0);
            return 0;
        }
        let cm;
        match lowband {
            None => {
                for v in &mut x[..n] {
                    self.seed = lcg_rand(self.seed);
                    *v = ((self.seed as i32) >> 20) as f32;
                }
                cm = cm_mask;
            }
            Some(lb) => {
                for (v, &l) in x[..n].iter_mut().zip(lb) {
                    self.seed = lcg_rand(self.seed);
                    let tmp = if self.seed & 0x8000 != 0 {
                        1.0 / 256.0
                    } else {
                        -1.0 / 256.0
                    };
                    *v = l + tmp;
                }
                cm = fill;
            }
        }
        renormalise_vector(&mut x[..n], gain);
        cm
    }

    #[allow(clippy::too_many_arguments)]
    fn quant_band(
        &mut self,
        x: &mut [f32],
        n: usize,
        b: i32,
        mut bb: i32,
        lowband: Option<&[f32]>,
        lm: i32,
        lowband_out: Option<&mut [f32]>,
        gain: f32,
        mut fill: u32,
        scratch: &mut [f32; MAX_BAND],
    ) -> u32 {
        let n0 = n;
        let mut n_b = n / bb as usize;
        let long_blocks = bb == 1;
        let mut tf_change = self.tf_change;
        if n == 1 {
            return self.quant_band_n1(x, None, lowband_out);
        }
        let recombine = tf_change.max(0) as usize;
        let mut lb: Option<&mut [f32]> = match lowband {
            Some(l) => {
                scratch[..n].copy_from_slice(&l[..n]);
                Some(&mut scratch[..n])
            }
            None => None,
        };
        const BIT_INTERLEAVE: [u8; 16] = [0, 1, 1, 1, 2, 3, 3, 3, 2, 3, 3, 3, 2, 3, 3, 3];
        for k in 0..recombine {
            if let Some(l) = lb.as_deref_mut() {
                haar1(l, n >> k, 1 << k);
            }
            fill = u32::from(BIT_INTERLEAVE[(fill & 0xf) as usize])
                | u32::from(BIT_INTERLEAVE[(fill >> 4) as usize & 0xf]) << 2;
        }
        bb >>= recombine;
        n_b <<= recombine;
        let mut time_divide = 0;
        while n_b & 1 == 0 && tf_change < 0 {
            if let Some(l) = lb.as_deref_mut() {
                haar1(l, n_b, bb as usize);
            }
            fill |= fill << bb;
            bb <<= 1;
            n_b >>= 1;
            time_divide += 1;
            tf_change += 1;
        }
        let b0 = bb;
        let n_b0 = n_b;
        if b0 > 1 {
            if let Some(l) = lb.as_deref_mut() {
                deinterleave_hadamard(
                    &mut self.tmp,
                    l,
                    n_b >> recombine,
                    (b0 as usize) << recombine,
                    long_blocks,
                );
            }
        }
        let mut cm = self.quant_partition(x, n, b, bb, lb.as_deref(), lm, gain, fill);
        if b0 > 1 {
            interleave_hadamard(
                &mut self.tmp,
                x,
                n_b >> recombine,
                (b0 as usize) << recombine,
                long_blocks,
            );
        }
        n_b = n_b0;
        bb = b0;
        for _ in 0..time_divide {
            bb >>= 1;
            n_b <<= 1;
            cm |= cm >> bb;
            haar1(x, n_b, bb as usize);
        }
        const BIT_DEINTERLEAVE: [u8; 16] = [
            0x00, 0x03, 0x0C, 0x0F, 0x30, 0x33, 0x3C, 0x3F, 0xC0, 0xC3, 0xCC, 0xCF, 0xF0, 0xF3,
            0xFC, 0xFF,
        ];
        for k in 0..recombine {
            cm = u32::from(BIT_DEINTERLEAVE[cm as usize & 0xf]);
            haar1(x, n0 >> k, 1 << k);
        }
        bb <<= recombine;
        if let Some(out) = lowband_out {
            let nn = (n0 as f32).sqrt();
            for (o, &v) in out[..n0].iter_mut().zip(&x[..n0]) {
                *o = nn * v;
            }
        }
        cm & ((1u32 << bb) - 1)
    }

    #[allow(clippy::too_many_arguments)]
    fn quant_band_stereo(
        &mut self,
        x: &mut [f32],
        y: &mut [f32],
        n: usize,
        mut b: i32,
        bb: i32,
        lowband: Option<&[f32]>,
        lm: i32,
        lowband_out: Option<&mut [f32]>,
        mut fill: u32,
        scratch: &mut [f32; MAX_BAND],
    ) -> u32 {
        if n == 1 {
            return self.quant_band_n1(x, Some(y), lowband_out);
        }
        let orig_fill = fill;
        let s = self.compute_theta(n as i32, &mut b, bb, bb, lm, true, &mut fill);
        let mid = (1.0 / 32768.0) * s.imid as f32;
        let side = (1.0 / 32768.0) * s.iside as f32;
        let itheta = s.itheta;
        let mut cm;
        if n == 2 {
            let mut mbits = b;
            let mut sbits = 0;
            if itheta != 0 && itheta != 16384 {
                sbits = 1 << BITRES;
            }
            mbits -= sbits;
            let swap = itheta > 8192;
            self.remaining_bits -= s.qalloc + sbits;
            let mut sign = 0;
            if sbits != 0 {
                sign = self.dec.bits(1) as i32;
            }
            let sign = (1 - 2 * sign) as f32;
            let (x2, y2) = if swap {
                (&mut *y, &mut *x)
            } else {
                (&mut *x, &mut *y)
            };
            cm = self.quant_band(
                x2,
                n,
                mbits,
                bb,
                lowband,
                lm,
                lowband_out,
                1.0,
                orig_fill,
                scratch,
            );
            y2[0] = -sign * x2[1];
            y2[1] = sign * x2[0];
            x[0] *= mid;
            x[1] *= mid;
            y[0] *= side;
            y[1] *= side;
            let tmp = x[0];
            x[0] = tmp - y[0];
            y[0] = tmp + y[0];
            let tmp = x[1];
            x[1] = tmp - y[1];
            y[1] = tmp + y[1];
        } else {
            let mut mbits = b.min((b - s.delta) / 2).max(0);
            let mut sbits = b - mbits;
            self.remaining_bits -= s.qalloc;
            let mut rebalance = self.remaining_bits;
            if mbits >= sbits {
                cm = self.quant_band(
                    x,
                    n,
                    mbits,
                    bb,
                    lowband,
                    lm,
                    lowband_out,
                    1.0,
                    fill,
                    scratch,
                );
                rebalance = mbits - (rebalance - self.remaining_bits);
                if rebalance > 3 << BITRES && itheta != 0 {
                    sbits += rebalance - (3 << BITRES);
                }
                cm |= self.quant_band(y, n, sbits, bb, None, lm, None, side, fill >> bb, scratch);
            } else {
                cm = self.quant_band(y, n, sbits, bb, None, lm, None, side, fill >> bb, scratch);
                rebalance = sbits - (rebalance - self.remaining_bits);
                if rebalance > 3 << BITRES && itheta != 16384 {
                    mbits += rebalance - (3 << BITRES);
                }
                cm |= self.quant_band(
                    x,
                    n,
                    mbits,
                    bb,
                    lowband,
                    lm,
                    lowband_out,
                    1.0,
                    fill,
                    scratch,
                );
            }
            stereo_merge(&mut x[..n], &mut y[..n], mid);
        }
        if s.inv {
            for v in &mut y[..n] {
                *v = -*v;
            }
        }
        cm
    }
}

fn stereo_merge(x: &mut [f32], y: &mut [f32], mid: f32) {
    let mut xp = 0f32;
    let mut side = 0f32;
    for (&a, &b) in y.iter().zip(x.iter()) {
        xp += a * b;
        side += a * a;
    }
    xp *= mid;
    let el = mid * mid + side - 2.0 * xp;
    let er = mid * mid + side + 2.0 * xp;
    if er < 6e-4 || el < 6e-4 {
        y.copy_from_slice(x);
        return;
    }
    let lgain = 1.0 / el.sqrt();
    let rgain = 1.0 / er.sqrt();
    for (a, b) in x.iter_mut().zip(y.iter_mut()) {
        let l = mid * *a;
        let r = *b;
        *a = lgain * (l - r);
        *b = rgain * (l + r);
    }
}

/// Everything `quant_all_bands` needs besides the coder.
pub struct AllBands<'p> {
    pub start: usize,
    pub end: usize,
    pub pulses: &'p [i32; NB_EBANDS],
    pub short_blocks: bool,
    pub spread: usize,
    pub dual_stereo: bool,
    pub intensity: usize,
    pub tf_res: &'p [i32; NB_EBANDS],
    pub total_bits: i32,
    pub balance: i32,
    pub lm: usize,
    pub coded_bands: usize,
    pub disable_inv: bool,
}

/// Decode all band shapes into `x` (and `y` for stereo). Returns the
/// per-band collapse masks (`band * C + channel`).
pub fn quant_all_bands(
    p: &AllBands,
    x_: &mut [f32],
    mut y_: Option<&mut [f32]>,
    collapse_masks: &mut [u8; 2 * NB_EBANDS],
    dec: &mut RangeDecoder,
    seed: &mut u32,
) {
    let m = 1usize << p.lm;
    let c = if y_.is_some() { 2 } else { 1 };
    let bb: i32 = if p.short_blocks { m as i32 } else { 1 };
    let eb = |i: usize| m * EBANDS[i] as usize;
    let norm_offset = eb(p.start);
    let norm_len = eb(NB_EBANDS - 1) - norm_offset;
    // Largest case: 8 * 78 bins per channel (start band 0, LM 3).
    let mut norm_buf = [0f32; 2 * 624];
    let (norm, norm2) = norm_buf[..2 * norm_len].split_at_mut(norm_len);
    let mut ctx = BandCtx {
        dec,
        i: 0,
        intensity: p.intensity,
        spread: p.spread,
        tf_change: 0,
        remaining_bits: 0,
        seed: *seed,
        disable_inv: p.disable_inv,
        avoid_split_noise: bb > 1,
        iy: [0; MAX_BAND],
        tmp: [0.0; MAX_BAND],
    };
    let mut scratch = [0f32; MAX_BAND];
    let mut lowband_offset = 0usize;
    let mut update_lowband = true;
    let mut dual_stereo = p.dual_stereo;
    let mut balance = p.balance;
    let mut lb = [0f32; MAX_BAND];
    let mut lb2 = [0f32; MAX_BAND];
    for i in p.start..p.end {
        ctx.i = i;
        let last = i == p.end - 1;
        let n = eb(i + 1) - eb(i);
        let tell = ctx.dec.tell_frac() as i32;
        if i != p.start {
            balance -= tell;
        }
        let remaining_bits = p.total_bits - tell - 1;
        ctx.remaining_bits = remaining_bits;
        let b = if i < p.coded_bands {
            let curr_balance = sdiv(balance, 3.min((p.coded_bands - i) as i32));
            0.max(16383.min((remaining_bits + 1).min(p.pulses[i] + curr_balance)))
        } else {
            0
        };
        if (eb(i) as isize - n as isize >= eb(p.start) as isize || i == p.start + 1)
            && (update_lowband || lowband_offset == 0)
        {
            lowband_offset = i;
        }
        if i == p.start + 1 {
            // Special hybrid folding: duplicate the first band's folding data.
            let n1 = eb(p.start + 1) - eb(p.start);
            let n2 = eb(p.start + 2) - eb(p.start + 1);
            if n2 > n1 {
                norm.copy_within(2 * n1 - n2..n1, n1);
                if dual_stereo {
                    norm2.copy_within(2 * n1 - n2..n1, n1);
                }
            }
        }
        ctx.tf_change = p.tf_res[i];
        let mut effective_lowband: Option<usize> = None;
        let (mut x_cm, mut y_cm);
        if lowband_offset != 0 && (p.spread != 3 || bb > 1 || ctx.tf_change < 0) {
            let el =
                (eb(lowband_offset) as isize - norm_offset as isize - n as isize).max(0) as usize;
            effective_lowband = Some(el);
            let mut fold_start = lowband_offset;
            loop {
                fold_start -= 1;
                if eb(fold_start) <= el + norm_offset {
                    break;
                }
            }
            let mut fold_end = lowband_offset - 1;
            loop {
                fold_end += 1;
                if !(fold_end < i && eb(fold_end) < el + norm_offset + n) {
                    break;
                }
            }
            x_cm = 0u32;
            y_cm = 0u32;
            let mut fi = fold_start;
            loop {
                x_cm |= u32::from(collapse_masks[fi * c]);
                y_cm |= u32::from(collapse_masks[fi * c + c - 1]);
                fi += 1;
                if fi >= fold_end {
                    break;
                }
            }
        } else {
            x_cm = (1u32 << bb) - 1;
            y_cm = x_cm;
        }
        if dual_stereo && i == p.intensity {
            dual_stereo = false;
            let lim = eb(i) - norm_offset;
            for j in 0..lim {
                norm[j] = 0.5 * (norm[j] + norm2[j]);
            }
        }
        let xr = &mut x_[eb(i)..eb(i) + n];
        let out_at = eb(i) - norm_offset;
        if let Some(el) = effective_lowband {
            lb[..n].copy_from_slice(&norm[el..el + n]);
        }
        let lowband = effective_lowband.map(|_| &lb[..n]);
        if dual_stereo {
            if let Some(el) = effective_lowband {
                lb2[..n].copy_from_slice(&norm2[el..el + n]);
            }
            let lowband2 = effective_lowband.map(|_| &lb2[..n]);
            let yr = &mut y_.as_deref_mut().expect("stereo")[eb(i)..eb(i) + n];
            x_cm = ctx.quant_band(
                xr,
                n,
                b / 2,
                bb,
                lowband,
                p.lm as i32,
                if last {
                    None
                } else {
                    Some(&mut norm[out_at..out_at + n])
                },
                1.0,
                x_cm,
                &mut scratch,
            );
            y_cm = ctx.quant_band(
                yr,
                n,
                b / 2,
                bb,
                lowband2,
                p.lm as i32,
                if last {
                    None
                } else {
                    Some(&mut norm2[out_at..out_at + n])
                },
                1.0,
                y_cm,
                &mut scratch,
            );
        } else {
            let out = if last {
                None
            } else {
                Some(&mut norm[out_at..out_at + n])
            };
            if let Some(y_all) = y_.as_deref_mut() {
                let yr = &mut y_all[eb(i)..eb(i) + n];
                x_cm = ctx.quant_band_stereo(
                    xr,
                    yr,
                    n,
                    b,
                    bb,
                    lowband,
                    p.lm as i32,
                    out,
                    x_cm | y_cm,
                    &mut scratch,
                );
            } else {
                x_cm = ctx.quant_band(
                    xr,
                    n,
                    b,
                    bb,
                    lowband,
                    p.lm as i32,
                    out,
                    1.0,
                    x_cm | y_cm,
                    &mut scratch,
                );
            }
            y_cm = x_cm;
        }
        collapse_masks[i * c] = x_cm as u8;
        collapse_masks[i * c + c - 1] = y_cm as u8;
        balance += p.pulses[i] + tell;
        update_lowband = b > (n as i32) << BITRES;
        ctx.avoid_split_noise = false;
    }
    *seed = ctx.seed;
}

/// Fill collapsed short blocks with noise (RFC 6716 4.3.5).
#[allow(clippy::too_many_arguments)]
pub fn anti_collapse(
    x_: &mut [f32],
    collapse_masks: &[u8; 2 * NB_EBANDS],
    lm: usize,
    c: usize,
    size: usize,
    start: usize,
    end: usize,
    log_e: &[f32; 2 * NB_EBANDS],
    prev1: &[f32; 2 * NB_EBANDS],
    prev2: &[f32; 2 * NB_EBANDS],
    pulses: &[i32; NB_EBANDS],
    mut seed: u32,
) {
    for i in start..end {
        let n0 = (EBANDS[i + 1] - EBANDS[i]) as usize;
        let depth = ((1 + pulses[i]) as u32 / n0 as u32) >> lm;
        let thresh = 0.5 * exp2(-0.125 * depth as f32);
        let sqrt_1 = 1.0 / (((n0 << lm) as f32).sqrt());
        for ch in 0..c {
            let mut p1 = prev1[ch * NB_EBANDS + i];
            let mut p2 = prev2[ch * NB_EBANDS + i];
            if c == 1 {
                p1 = p1.max(prev1[NB_EBANDS + i]);
                p2 = p2.max(prev2[NB_EBANDS + i]);
            }
            let ediff = (log_e[ch * NB_EBANDS + i] - p1.min(p2)).max(0.0);
            let mut r = 2.0 * exp2(-ediff);
            if lm == 3 {
                r *= std::f32::consts::SQRT_2;
            }
            r = thresh.min(r);
            r *= sqrt_1;
            let base = ch * size + ((EBANDS[i] as usize) << lm);
            let x = &mut x_[base..base + (n0 << lm)];
            let mut renormalize = false;
            for k in 0..1usize << lm {
                if collapse_masks[i * c + ch] & (1 << k) == 0 {
                    for j in 0..n0 {
                        seed = lcg_rand(seed);
                        x[(j << lm) + k] = if seed & 0x8000 != 0 { r } else { -r };
                    }
                    renormalize = true;
                }
            }
            if renormalize {
                renormalise_vector(x, 1.0);
            }
        }
    }
}

/// `2^x` matching the reference float build (`exp(ln2 * x)` in double).
#[inline]
pub fn exp2(x: f32) -> f32 {
    (std::f64::consts::LN_2 * f64::from(x)).exp() as f32
}

/// Scale normalised bands by their energies into MDCT coefficients.
#[allow(clippy::too_many_arguments)]
pub fn denormalise_bands(
    x: &[f32],
    freq: &mut [f32],
    band_log_e: &[f32],
    start: usize,
    end: usize,
    m: usize,
    downsample: usize,
    silence: bool,
) {
    let n = m * SHORT_MDCT_SIZE;
    let mut bound = m * EBANDS[end] as usize;
    if downsample != 1 {
        bound = bound.min(n / downsample);
    }
    let (start, end) = if silence {
        bound = 0;
        (0, 0)
    } else {
        (start, end)
    };
    let s0 = m * EBANDS[start] as usize;
    freq[..s0].fill(0.0);
    for i in start..end {
        let j0 = m * EBANDS[i] as usize;
        let j1 = m * EBANDS[i + 1] as usize;
        let lg = band_log_e[i] + E_MEANS[i];
        let g = exp2(lg.min(32.0));
        for j in j0..j1 {
            freq[j] = x[j] * g;
        }
    }
    freq[bound..n].fill(0.0);
}
