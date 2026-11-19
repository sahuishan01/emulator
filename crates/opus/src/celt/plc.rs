//! Signal helpers for CELT packet-loss concealment: pitch search and LPC.

pub const LPC_ORDER: usize = 24;

/// `ac[k] = sum x[i] x[i-k]` with optional symmetric windowing, computed in
/// the same split order as the reference (bulk correlation, then tail).
pub fn autocorr(x: &[f32], ac: &mut [f32], window: Option<&[f32]>, overlap: usize, lag: usize) {
    let n = x.len();
    let mut xx;
    let xp: &[f32] = if let Some(w) = window.filter(|_| overlap > 0) {
        xx = x.to_vec();
        for i in 0..overlap {
            xx[i] = x[i] * w[i];
            xx[n - i - 1] = x[n - i - 1] * w[i];
        }
        &xx
    } else {
        x
    };
    let fast_n = n - lag;
    for k in 0..=lag {
        let mut s = 0f32;
        for j in 0..fast_n {
            s += xp[j] * xp[j + k];
        }
        let mut d = 0f32;
        for i in k + fast_n..n {
            d += xp[i] * xp[i - k];
        }
        ac[k] = s + d;
    }
}

/// Levinson-Durbin recursion; `lpc.len()` is the order.
pub fn lpc(lpc: &mut [f32], ac: &[f32]) {
    let p = lpc.len();
    lpc.fill(0.0);
    let mut error = ac[0];
    if ac[0] > 1e-10 {
        for i in 0..p {
            let mut rr = 0f32;
            for j in 0..i {
                rr += lpc[j] * ac[i - j];
            }
            rr += ac[i + 1];
            let r = -(rr / error);
            lpc[i] = r;
            for j in 0..(i + 1) >> 1 {
                let t1 = lpc[j];
                let t2 = lpc[i - 1 - j];
                lpc[j] = t1 + r * t2;
                lpc[i - 1 - j] = t2 + r * t1;
            }
            error -= r * r * error;
            if error <= 0.001 * ac[0] {
                break;
            }
        }
    }
}

/// FIR `y[i] = x[i] + sum num[k] x[i-1-k]`; `x` has `ord` history samples
/// before `start`.
pub fn fir(x: &[f32], start: usize, num: &[f32], y: &mut [f32]) {
    let ord = num.len();
    for (i, out) in y.iter_mut().enumerate() {
        let mut sum = x[start + i];
        for j in 0..ord {
            sum += num[ord - 1 - j] * x[start + i + j - ord];
        }
        *out = sum;
    }
}

/// IIR `y[i] = x[i] - sum den[k] y[i-1-k]` in place; `mem[k] = y[-1-k]`.
pub fn iir(buf: &mut [f32], den: &[f32], mem: &mut [f32]) {
    let ord = den.len();
    let n = buf.len();
    let mut y = vec![0f32; n + ord];
    for i in 0..ord {
        y[i] = mem[ord - i - 1];
    }
    for i in 0..n {
        let mut sum = buf[i];
        for j in 0..ord {
            sum -= den[ord - 1 - j] * y[i + j];
        }
        y[i + ord] = sum;
        buf[i] = sum;
    }
    for i in 0..ord {
        mem[i] = buf[n - i - 1];
    }
}

fn find_best_pitch(xcorr: &[f32], y: &[f32], len: usize, max_pitch: usize, best: &mut [usize; 2]) {
    let mut syy = 1f32;
    let mut best_num = [-1f32; 2];
    let mut best_den = [0f32; 2];
    *best = [0, 1];
    for v in &y[..len] {
        syy += v * v;
    }
    for i in 0..max_pitch {
        if xcorr[i] > 0.0 {
            let x16 = xcorr[i] * 1e-12;
            let num = x16 * x16;
            if num * best_den[1] > best_num[1] * syy {
                if num * best_den[0] > best_num[0] * syy {
                    best_num[1] = best_num[0];
                    best_den[1] = best_den[0];
                    best[1] = best[0];
                    best_num[0] = num;
                    best_den[0] = syy;
                    best[0] = i;
                } else {
                    best_num[1] = num;
                    best_den[1] = syy;
                    best[1] = i;
                }
            }
        }
        syy += y[i + len] * y[i + len] - y[i] * y[i];
        syy = syy.max(1.0);
    }
}

/// 2x decimation with a 4-tap LPC whitening filter, mixing channels.
pub fn pitch_downsample(x: &[&[f32]], x_lp: &mut [f32], len: usize) {
    let half = len >> 1;
    for i in 1..half {
        x_lp[i] = 0.25 * x[0][2 * i - 1] + 0.25 * x[0][2 * i + 1] + 0.5 * x[0][2 * i];
    }
    x_lp[0] = 0.25 * x[0][1] + 0.5 * x[0][0];
    if x.len() == 2 {
        for i in 1..half {
            x_lp[i] += 0.25 * x[1][2 * i - 1] + 0.25 * x[1][2 * i + 1] + 0.5 * x[1][2 * i];
        }
        x_lp[0] += 0.25 * x[1][1] + 0.5 * x[1][0];
    }
    let mut ac = [0f32; 5];
    autocorr(&x_lp[..half], &mut ac, None, 0, 4);
    ac[0] *= 1.0001;
    for (i, a) in ac.iter_mut().enumerate().skip(1) {
        let f = 0.008 * i as f32;
        *a -= *a * f * f;
    }
    let mut l = [0f32; 4];
    lpc(&mut l, &ac);
    let mut tmp = 1.0f32;
    for v in &mut l {
        tmp *= 0.9;
        *v *= tmp;
    }
    let c1 = 0.8f32;
    let num = [
        l[0] + 0.8,
        l[1] + c1 * l[0],
        l[2] + c1 * l[1],
        l[3] + c1 * l[2],
        c1 * l[3],
    ];
    let mut mem = [0f32; 5];
    for v in &mut x_lp[..half] {
        let sum = *v
            + num[0] * mem[0]
            + num[1] * mem[1]
            + num[2] * mem[2]
            + num[3] * mem[3]
            + num[4] * mem[4];
        mem = [*v, mem[0], mem[1], mem[2], mem[3]];
        *v = sum;
    }
}

fn inner(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Open-loop pitch search on the decimated signal; returns the lag.
pub fn pitch_search(x_lp: &[f32], y: &[f32], len: usize, max_pitch: usize) -> usize {
    let lag = len + max_pitch;
    let x4: Vec<f32> = (0..len >> 2).map(|j| x_lp[2 * j]).collect();
    let y4: Vec<f32> = (0..lag >> 2).map(|j| y[2 * j]).collect();
    let mut xcorr = vec![0f32; max_pitch >> 1];
    for i in 0..max_pitch >> 2 {
        xcorr[i] = inner(&x4, &y4[i..i + (len >> 2)]);
    }
    let mut best = [0usize; 2];
    find_best_pitch(&xcorr, &y4, len >> 2, max_pitch >> 2, &mut best);
    for i in 0..max_pitch >> 1 {
        xcorr[i] = 0.0;
        if (i as isize - 2 * best[0] as isize).abs() > 2
            && (i as isize - 2 * best[1] as isize).abs() > 2
        {
            continue;
        }
        let sum = inner(&x_lp[..len >> 1], &y[i..i + (len >> 1)]);
        xcorr[i] = sum.max(-1.0);
    }
    find_best_pitch(&xcorr, y, len >> 1, max_pitch >> 1, &mut best);
    let offset = if best[0] > 0 && best[0] < (max_pitch >> 1) - 1 {
        let a = xcorr[best[0] - 1];
        let b = xcorr[best[0]];
        let c = xcorr[best[0] + 1];
        if c - a > 0.7 * (b - a) {
            1
        } else if a - c > 0.7 * (b - c) {
            -1
        } else {
            0
        }
    } else {
        0
    };
    (2 * best[0] as isize - offset) as usize
}
