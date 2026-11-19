//! Inverse MDCT via an N/4-point complex FFT, plus the CELT window.
//!
//! The FFT is an original Stockham autosort mixed-radix (4, 2, 3, 5)
//! implementation with precomputed twiddles; it needs no bit reversal and
//! works out of place between two buffers.

use std::sync::OnceLock;

use super::tables::{MAX_LM, OVERLAP, SHORT_MDCT_SIZE};

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    #[inline]
    fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
    #[inline]
    fn add(self, o: Self) -> Self {
        Self::new(self.re + o.re, self.im + o.im)
    }
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self::new(self.re - o.re, self.im - o.im)
    }
    #[inline]
    fn mul(self, o: Self) -> Self {
        Self::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
    #[inline]
    fn scale(self, s: f32) -> Self {
        Self::new(self.re * s, self.im * s)
    }
    /// Multiply by -i.
    #[inline]
    fn mul_neg_i(self) -> Self {
        Self::new(self.im, -self.re)
    }
}

/// Forward (unnormalised, `e^{-2 pi i nk/N}`) FFT plan.
pub struct Fft {
    n: usize,
    /// (radix, offset into `twiddles`) per stage.
    stages: Vec<(usize, usize)>,
    /// Per stage and output group q: `w^q, w^2q, ... w^(p-1)q` stored
    /// contiguously so the butterflies stream through them.
    twiddles: Vec<Complex>,
}

impl Fft {
    pub fn new(n: usize) -> Self {
        let mut factors = Vec::new();
        let mut m = n;
        for p in [4, 2, 3, 5] {
            while m % p == 0 {
                factors.push(p);
                m /= p;
            }
        }
        assert_eq!(m, 1, "FFT size {n} has unsupported factors");
        let mut stages = Vec::new();
        let mut twiddles = Vec::new();
        let mut len = n;
        for &p in &factors {
            stages.push((p, twiddles.len()));
            let m = len / p;
            for q in 0..m {
                for r in 1..p {
                    let a = -2.0 * std::f64::consts::PI * (q * r) as f64 / len as f64;
                    twiddles.push(Complex::new(a.cos() as f32, a.sin() as f32));
                }
            }
            len = m;
        }
        Self {
            n,
            stages,
            twiddles,
        }
    }

    /// Transform `a` using `b` as the ping-pong buffer. Returns `true` when
    /// the result ended up in `b` (odd number of stages).
    pub fn process(&self, a: &mut [Complex], b: &mut [Complex]) -> bool {
        let n = self.n;
        let mut len = n;
        let mut stride = 1;
        let mut in_a = true;
        for &(p, tw) in &self.stages {
            let (src, dst): (&[Complex], &mut [Complex]) = if in_a {
                (&a[..n], &mut b[..n])
            } else {
                (&b[..n], &mut a[..n])
            };
            let m = len / p;
            stockham_stage(src, dst, m, stride, p, &self.twiddles[tw..tw + m * (p - 1)]);
            len = m;
            stride *= p;
            in_a = !in_a;
        }
        !in_a
    }
}

#[inline(always)]
fn bfly2(a: Complex, b: Complex, w1: Complex) -> [Complex; 2] {
    [a.add(b), a.sub(b).mul(w1)]
}

#[inline(always)]
fn bfly4(x0: Complex, x1: Complex, x2: Complex, x3: Complex, w: &[Complex]) -> [Complex; 4] {
    let t0 = x0.add(x2);
    let t1 = x0.sub(x2);
    let t2 = x1.add(x3);
    let t3 = x1.sub(x3).mul_neg_i();
    [
        t0.add(t2),
        t1.add(t3).mul(w[0]),
        t0.sub(t2).mul(w[1]),
        t1.sub(t3).mul(w[2]),
    ]
}

#[inline(always)]
fn bfly3(a0: Complex, x1: Complex, x2: Complex, w: &[Complex]) -> [Complex; 3] {
    const C: f32 = -0.5;
    const SN: f32 = -0.866_025_4;
    let sum = x1.add(x2);
    let dif = x1.sub(x2);
    let mid = a0.add(sum.scale(C));
    let rot = Complex::new(-dif.im * SN, dif.re * SN);
    [a0.add(sum), mid.add(rot).mul(w[0]), mid.sub(rot).mul(w[1])]
}

#[inline(always)]
fn bfly5(
    x0: Complex,
    x1: Complex,
    x2: Complex,
    x3: Complex,
    x4: Complex,
    w: &[Complex],
) -> [Complex; 5] {
    // cos/sin of 2*pi/5 and 4*pi/5.
    const C1: f32 = 0.309_017;
    const S1: f32 = -0.951_056_5;
    const C2: f32 = -0.809_017;
    const S2: f32 = -0.587_785_24;
    let s14 = x1.add(x4);
    let d14 = x1.sub(x4);
    let s23 = x2.add(x3);
    let d23 = x2.sub(x3);
    let r1 = x0.add(s14.scale(C1)).add(s23.scale(C2));
    let r2 = x0.add(s14.scale(C2)).add(s23.scale(C1));
    let j1 = d14.scale(S1).add(d23.scale(S2));
    let j2 = d14.scale(S2).sub(d23.scale(S1));
    let i1 = Complex::new(-j1.im, j1.re);
    let i2 = Complex::new(-j2.im, j2.re);
    [
        x0.add(s14).add(s23),
        r1.add(i1).mul(w[0]),
        r2.add(i2).mul(w[1]),
        r2.sub(i2).mul(w[2]),
        r1.sub(i1).mul(w[3]),
    ]
}

/// One Stockham DIF stage: `p` groups of `m * s` inputs to `m` groups of
/// `p * s` outputs, twiddled by `w^(q r)`.
#[inline]
fn stockham_stage(
    src: &[Complex],
    dst: &mut [Complex],
    m: usize,
    s: usize,
    p: usize,
    tw: &[Complex],
) {
    let ms = m * s;
    match p {
        2 => {
            for q in 0..m {
                let w1 = tw[q];
                let x0 = &src[s * q..s * q + s];
                let x1 = &src[ms + s * q..ms + s * q + s];
                let (d0, d1) = dst[2 * s * q..2 * s * q + 2 * s].split_at_mut(s);
                for k in 0..s {
                    [d0[k], d1[k]] = bfly2(x0[k], x1[k], w1);
                }
            }
        }
        4 if s == 1 => {
            let (x0, rest) = src[..4 * m].split_at(m);
            let (x1, rest) = rest.split_at(m);
            let (x2, x3) = rest.split_at(m);
            for (q, (d, w)) in dst[..4 * m]
                .chunks_exact_mut(4)
                .zip(tw.chunks_exact(3))
                .enumerate()
            {
                let r = bfly4(x0[q], x1[q], x2[q], x3[q], w);
                d.copy_from_slice(&r);
            }
        }
        4 => {
            for q in 0..m {
                let w = &tw[3 * q..3 * q + 3];
                let x0 = &src[s * q..s * q + s];
                let x1 = &src[ms + s * q..ms + s * q + s];
                let x2 = &src[2 * ms + s * q..2 * ms + s * q + s];
                let x3 = &src[3 * ms + s * q..3 * ms + s * q + s];
                let out = &mut dst[s * 4 * q..s * 4 * q + 4 * s];
                let (d0, rest) = out.split_at_mut(s);
                let (d1, rest) = rest.split_at_mut(s);
                let (d2, d3) = rest.split_at_mut(s);
                for k in 0..s {
                    [d0[k], d1[k], d2[k], d3[k]] = bfly4(x0[k], x1[k], x2[k], x3[k], w);
                }
            }
        }
        3 => {
            for q in 0..m {
                let w = &tw[2 * q..2 * q + 2];
                let x0 = &src[s * q..s * q + s];
                let x1 = &src[ms + s * q..ms + s * q + s];
                let x2 = &src[2 * ms + s * q..2 * ms + s * q + s];
                let out = &mut dst[s * 3 * q..s * 3 * q + 3 * s];
                let (d0, rest) = out.split_at_mut(s);
                let (d1, d2) = rest.split_at_mut(s);
                for k in 0..s {
                    [d0[k], d1[k], d2[k]] = bfly3(x0[k], x1[k], x2[k], w);
                }
            }
        }
        5 => {
            for q in 0..m {
                let w = &tw[4 * q..4 * q + 4];
                let x0 = &src[s * q..s * q + s];
                let x1 = &src[ms + s * q..ms + s * q + s];
                let x2 = &src[2 * ms + s * q..2 * ms + s * q + s];
                let x3 = &src[3 * ms + s * q..3 * ms + s * q + s];
                let x4 = &src[4 * ms + s * q..4 * ms + s * q + s];
                let out = &mut dst[s * 5 * q..s * 5 * q + 5 * s];
                let (d0, rest) = out.split_at_mut(s);
                let (d1, rest) = rest.split_at_mut(s);
                let (d2, rest) = rest.split_at_mut(s);
                let (d3, d4) = rest.split_at_mut(s);
                for k in 0..s {
                    [d0[k], d1[k], d2[k], d3[k], d4[k]] =
                        bfly5(x0[k], x1[k], x2[k], x3[k], x4[k], w);
                }
            }
        }
        _ => unreachable!("radix {p}"),
    }
}

/// Static MDCT data for the 48 kHz mode (MDCT size 1920 >> shift).
pub struct Mdct {
    trig: [Vec<f32>; MAX_LM + 1],
    fft: [Fft; MAX_LM + 1],
    pub window: [f32; OVERLAP],
}

pub fn mdct() -> &'static Mdct {
    static MDCT: OnceLock<Mdct> = OnceLock::new();
    MDCT.get_or_init(|| {
        let n0 = 2 * SHORT_MDCT_SIZE << MAX_LM;
        let trig = std::array::from_fn(|shift| {
            let n = n0 >> shift;
            (0..n / 2)
                .map(|i| (2.0 * std::f64::consts::PI * (i as f64 + 0.125) / n as f64).cos() as f32)
                .collect()
        });
        let fft = std::array::from_fn(|shift| Fft::new(n0 >> 2 >> shift));
        let mut window = [0f32; OVERLAP];
        for (i, w) in window.iter_mut().enumerate() {
            let x = (0.5 * std::f64::consts::PI * (i as f64 + 0.5) / OVERLAP as f64).sin();
            *w = (0.5 * std::f64::consts::PI * x * x).sin() as f32;
        }
        Mdct { trig, fft, window }
    })
}

/// Reusable buffers for the inverse transform.
#[derive(Clone)]
pub struct MdctScratch {
    a: Vec<Complex>,
    b: Vec<Complex>,
}

impl Default for MdctScratch {
    fn default() -> Self {
        let n = (2 * SHORT_MDCT_SIZE << MAX_LM) / 4;
        Self {
            a: vec![Complex::default(); n],
            b: vec![Complex::default(); n],
        }
    }
}

impl Mdct {
    /// Inverse MDCT of `input` (read with `stride`) into `out`, which must
    /// hold `overlap / 2 + n / 2` writable samples after any history. The
    /// first `OVERLAP` samples are combined with the previous block (TDAC).
    pub fn backward(
        &self,
        input: &[f32],
        out: &mut [f32],
        shift: usize,
        stride: usize,
        scratch: &mut MdctScratch,
    ) {
        let n = (2 * SHORT_MDCT_SIZE << MAX_LM) >> shift;
        let n2 = n >> 1;
        let n4 = n >> 2;
        let trig = &self.trig[shift];
        let (a, b) = (&mut scratch.a[..n4], &mut scratch.b[..n4]);
        // Pre-rotation (real/imaginary swapped: FFT used as an IFFT).
        for i in 0..n4 {
            let x1 = input[2 * i * stride];
            let x2 = input[stride * (n2 - 1 - 2 * i)];
            let yr = x2 * trig[i] + x1 * trig[n4 + i];
            let yi = x1 * trig[i] - x2 * trig[n4 + i];
            a[i] = Complex::new(yi, yr);
        }
        let z: &[Complex] = if self.fft[shift].process(a, b) { b } else { a };
        let half = OVERLAP >> 1;
        // Post-rotation from both ends, reading the FFT output directly.
        let y = &mut out[half..half + n2];
        for i in 0..(n4 + 1) >> 1 {
            let z0 = z[i];
            let z1 = z[n4 - 1 - i];
            let (re, im) = (z0.im, z0.re);
            let (t0, t1) = (trig[i], trig[n4 + i]);
            let yr0 = re * t0 + im * t1;
            let yi0 = re * t1 - im * t0;
            let (re2, im2) = (z1.im, z1.re);
            let (t0, t1) = (trig[n4 - i - 1], trig[n2 - i - 1]);
            let yr1 = re2 * t0 + im2 * t1;
            let yi1 = re2 * t1 - im2 * t0;
            y[2 * i] = yr0;
            y[n2 - 1 - 2 * i] = yi0;
            y[n2 - 2 - 2 * i] = yr1;
            y[2 * i + 1] = yi1;
        }
        // Mirror on both sides for TDAC.
        let w = &self.window;
        for i in 0..half {
            let x1 = out[OVERLAP - 1 - i];
            let x2 = out[i];
            out[i] = w[OVERLAP - 1 - i] * x2 - w[i] * x1;
            out[OVERLAP - 1 - i] = w[i] * x2 + w[OVERLAP - 1 - i] * x1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_dft(x: &[Complex]) -> Vec<Complex> {
        let n = x.len();
        (0..n)
            .map(|k| {
                let mut acc = (0f64, 0f64);
                for (t, v) in x.iter().enumerate() {
                    let a = -2.0 * std::f64::consts::PI * (t * k % n) as f64 / n as f64;
                    acc.0 += v.re as f64 * a.cos() - v.im as f64 * a.sin();
                    acc.1 += v.re as f64 * a.sin() + v.im as f64 * a.cos();
                }
                Complex::new(acc.0 as f32, acc.1 as f32)
            })
            .collect()
    }

    #[test]
    fn fft_matches_naive_dft_for_all_celt_sizes() {
        for n in [60usize, 120, 240, 480, 15, 8, 6, 10] {
            let fft = Fft::new(n);
            let mut x: Vec<Complex> = (0..n)
                .map(|i| {
                    Complex::new(
                        ((i * 7 + 3) % 11) as f32 - 5.0,
                        ((i * 5 + 1) % 13) as f32 - 6.0,
                    )
                })
                .collect();
            let want = naive_dft(&x);
            let mut s = vec![Complex::default(); n];
            let res = if fft.process(&mut x, &mut s) { s } else { x };
            for (a, b) in res.iter().zip(&want) {
                assert!(
                    (a.re - b.re).abs() < 1e-2 && (a.im - b.im).abs() < 1e-2,
                    "n={n}: {a:?} vs {b:?}"
                );
            }
        }
    }
}
