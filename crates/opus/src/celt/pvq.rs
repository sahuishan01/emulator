//! Pyramid vector quantiser decoding (RFC 6716 sections 4.3.4.1-4.3.4.3).
//!
//! `U(n, k)` counts the codewords of the PVQ codebook; a codeword index is
//! decoded dimension by dimension from the end of the table like the
//! reference enumeration. The table is computed once at start-up instead of
//! being stored.

use std::sync::OnceLock;

use crate::range::RangeDecoder;

/// Rows of `U(min(n,k), max(n,k))` that fit in 32 bits for every band
/// configuration (`min <= 14`, `max <= 176 + 2`).
const ROWS: usize = 15;
const COLS: usize = 180;

fn table() -> &'static [[u32; COLS]; ROWS] {
    static TABLE: OnceLock<Box<[[u32; COLS]; ROWS]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = Box::new([[0u32; COLS]; ROWS]);
        // U(0,0) = 1, U(0,k) = 0; row r is U(r, c) for c >= r.
        t[0][0] = 1;
        for r in 1..ROWS {
            for c in r..COLS {
                // U(r, c) = U(r-1, c) + U(r, c-1) + U(r-1, c-1).
                let up = t[r - 1][c];
                let diag = t[r - 1][c - 1];
                let left = if c > r { t[r][c - 1] } else { t[r - 1][c] };
                t[r][c] = up.wrapping_add(left).wrapping_add(diag);
            }
        }
        t
    })
}

#[inline]
fn u(t: &[[u32; COLS]; ROWS], n: usize, k: usize) -> u32 {
    let (a, b) = if n < k { (n, k) } else { (k, n) };
    t[a][b]
}

/// Number of codewords `V(n, k) = U(n, k) + U(n, k + 1)`.
pub fn codebook_size(n: usize, k: usize) -> u32 {
    let t = table();
    u(t, n, k).wrapping_add(u(t, n, k + 1))
}

/// Decode `k` pulses over `n > 1` dimensions into `y`; returns `sum(y^2)`.
pub fn decode_pulses(y: &mut [i32], n: usize, k: usize, dec: &mut RangeDecoder) -> f32 {
    let t = table();
    let mut i = dec.uint(codebook_size(n, k));
    let mut n = n;
    let mut k = k;
    let mut yy = 0i32;
    let mut out = 0usize;
    while n > 2 {
        let val;
        if k >= n {
            let row = &t[n];
            let p = row[k + 1];
            let s = if i >= p { -1i32 } else { 0 };
            if s != 0 {
                i -= p;
            }
            let k0 = k;
            let q = row[n];
            let mut p;
            if q > i {
                k = n;
                loop {
                    k -= 1;
                    p = t[k][n];
                    if p <= i {
                        break;
                    }
                }
            } else {
                p = row[k];
                while p > i {
                    k -= 1;
                    p = row[k];
                }
            }
            i -= p;
            val = ((k0 - k) as i32 + s) ^ s;
        } else {
            let p = t[k][n];
            let q = t[k + 1][n];
            if p <= i && i < q {
                i -= p;
                val = 0;
            } else {
                let s = if i >= q { -1i32 } else { 0 };
                if s != 0 {
                    i -= q;
                }
                let k0 = k;
                let mut p;
                loop {
                    k -= 1;
                    p = t[k][n];
                    if p <= i {
                        break;
                    }
                }
                i -= p;
                val = ((k0 - k) as i32 + s) ^ s;
            }
        }
        y[out] = val;
        out += 1;
        yy += val * val;
        n -= 1;
    }
    // n == 2
    let p = (2 * k + 1) as u32;
    let s = if i >= p { -1i32 } else { 0 };
    if s != 0 {
        i -= p;
    }
    let k0 = k;
    k = ((i + 1) >> 1) as usize;
    if k != 0 {
        i -= (2 * k - 1) as u32;
    }
    let val = ((k0 - k) as i32 + s) ^ s;
    y[out] = val;
    yy += val * val;
    // n == 1
    let s = -(i as i32);
    let val = (k as i32 + s) ^ s;
    y[out + 1] = val;
    yy += val * val;
    yy as f32
}

fn exp_rotation1(x: &mut [f32], len: usize, stride: usize, c: f32, s: f32) {
    let ms = -s;
    for i in 0..len - stride {
        let x1 = x[i];
        let x2 = x[i + stride];
        x[i + stride] = c * x2 + s * x1;
        x[i] = c * x1 + ms * x2;
    }
    if len >= 2 * stride + 1 {
        for i in (0..len - 2 * stride).rev() {
            let x1 = x[i];
            let x2 = x[i + stride];
            x[i + stride] = c * x2 + s * x1;
            x[i] = c * x1 + ms * x2;
        }
    }
}

/// Spreading rotation (RFC 6716 4.3.4.3). `dir < 0` is the decoder side.
pub fn exp_rotation(x: &mut [f32], len: usize, dir: i32, stride: usize, k: usize, spread: usize) {
    const SPREAD_FACTOR: [usize; 3] = [15, 10, 5];
    if 2 * k >= len || spread == 0 {
        return;
    }
    let factor = SPREAD_FACTOR[spread - 1];
    let gain = len as f32 / (len + factor * k) as f32;
    let theta = 0.5 * (gain * gain);
    let c = (0.5 * std::f32::consts::PI * theta).cos();
    let s = (0.5 * std::f32::consts::PI * (1.0 - theta)).cos();
    let mut stride2 = 0;
    if len >= 8 * stride {
        stride2 = 1;
        while (stride2 * stride2 + stride2) * stride + (stride >> 2) < len {
            stride2 += 1;
        }
    }
    let sub = len / stride;
    for i in 0..stride {
        let xs = &mut x[i * sub..(i + 1) * sub];
        if dir < 0 {
            if stride2 != 0 {
                exp_rotation1(xs, sub, stride2, s, c);
            }
            exp_rotation1(xs, sub, 1, c, s);
        } else {
            exp_rotation1(xs, sub, 1, c, -s);
            if stride2 != 0 {
                exp_rotation1(xs, sub, stride2, s, -c);
            }
        }
    }
}

/// Decode a PVQ band shape into `x` with norm `gain`; returns the collapse
/// mask (one bit per short block holding a pulse).
pub fn alg_unquant(
    x: &mut [f32],
    n: usize,
    k: usize,
    spread: usize,
    b: usize,
    dec: &mut RangeDecoder,
    gain: f32,
    iy: &mut [i32],
) -> u32 {
    let ryy = decode_pulses(iy, n, k, dec);
    let g = gain * (1.0 / ryy.sqrt());
    for (xo, &v) in x[..n].iter_mut().zip(&iy[..n]) {
        *xo = g * v as f32;
    }
    exp_rotation(x, n, -1, b, k, spread);
    if b <= 1 {
        return 1;
    }
    let n0 = n / b;
    let mut mask = 0;
    for i in 0..b {
        if iy[i * n0..(i + 1) * n0].iter().any(|&v| v != 0) {
            mask |= 1 << i;
        }
    }
    mask
}

/// Rescale `x` to norm `gain`.
pub fn renormalise_vector(x: &mut [f32], gain: f32) {
    let e = 1e-15 + x.iter().map(|v| v * v).sum::<f32>();
    let g = gain * (1.0 / e.sqrt());
    for v in x {
        *v *= g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codebook_sizes_match_closed_forms() {
        // V(n,1) = 2n, V(2,k) = 4k, V(3,2) = 18.
        assert_eq!(codebook_size(5, 1), 10);
        assert_eq!(codebook_size(2, 7), 28);
        assert_eq!(codebook_size(3, 2), 18);
        // V(N,K) values that must fit: N=8,K=15 as used by allocation caps.
        assert!(codebook_size(8, 15) > 0);
    }
}
