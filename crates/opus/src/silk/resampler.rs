//! SILK output resampler (internal 8/12/16 kHz to the API rate).
//!
//! Same structure and coefficients as the reference decoder so that SILK
//! output matches it exactly: 2x all-pass upsampling (+ 12-phase FIR
//! interpolation), or an AR2 + FIR decimator for lower output rates.

use super::macros::*;
use super::tables::*;

const UP2_HQ_0: [i32; 3] = [1746, 14986, 39083 - 65536];
const UP2_HQ_1: [i32; 3] = [6854, 25769, 55542 - 65536];
const ORDER_FIR_12: usize = 8;
const MAX_BATCH_MS: usize = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    Copy,
    Up2,
    IirFir,
    DownFir,
}

#[derive(Clone)]
pub struct Resampler {
    kind: Kind,
    s_iir: [i32; 6],
    s_fir_i16: [i16; 36],
    s_fir_i32: [i32; 36],
    delay_buf: [i16; 48],
    input_delay: usize,
    fs_in_khz: usize,
    fs_out_khz: usize,
    batch_size: usize,
    inv_ratio_q16: i32,
    fir_order: usize,
    fir_fracs: i32,
    coefs: &'static [i16],
}

impl Default for Resampler {
    fn default() -> Self {
        Self {
            kind: Kind::Copy,
            s_iir: [0; 6],
            s_fir_i16: [0; 36],
            s_fir_i32: [0; 36],
            delay_buf: [0; 48],
            input_delay: 0,
            fs_in_khz: 0,
            fs_out_khz: 0,
            batch_size: 0,
            inv_ratio_q16: 0,
            fir_order: 0,
            fir_fracs: 0,
            coefs: &[],
        }
    }
}

fn rate_id(r: u32) -> usize {
    match r {
        8000 => 0,
        12000 => 1,
        16000 => 2,
        24000 => 3,
        _ => 4,
    }
}

impl Resampler {
    pub fn new(fs_in: u32, fs_out: u32) -> Self {
        const DELAY_DEC: [[usize; 5]; 3] = [[4, 0, 2, 0, 0], [0, 9, 4, 7, 4], [0, 3, 12, 7, 7]];
        let mut s = Self {
            input_delay: DELAY_DEC[rate_id(fs_in)][rate_id(fs_out)],
            fs_in_khz: fs_in as usize / 1000,
            fs_out_khz: fs_out as usize / 1000,
            ..Self::default()
        };
        s.batch_size = s.fs_in_khz * MAX_BATCH_MS;
        let mut up2x = 0;
        if fs_out > fs_in {
            if fs_out == 2 * fs_in {
                s.kind = Kind::Up2;
            } else {
                s.kind = Kind::IirFir;
                up2x = 1;
            }
        } else if fs_out < fs_in {
            s.kind = Kind::DownFir;
            let (fracs, order, coefs): (i32, usize, &'static [i16]) = if fs_out * 4 == fs_in * 3 {
                (3, 18, &RESAMPLER_3_4_COEFS)
            } else if fs_out * 3 == fs_in * 2 {
                (2, 18, &RESAMPLER_2_3_COEFS)
            } else if fs_out * 2 == fs_in {
                (1, 24, &RESAMPLER_1_2_COEFS)
            } else if fs_out * 3 == fs_in {
                (1, 36, &RESAMPLER_1_3_COEFS)
            } else if fs_out * 4 == fs_in {
                (1, 36, &RESAMPLER_1_4_COEFS)
            } else {
                (1, 36, &RESAMPLER_1_6_COEFS)
            };
            s.fir_fracs = fracs;
            s.fir_order = order;
            s.coefs = coefs;
        } else {
            s.kind = Kind::Copy;
        }
        let fs_in = fs_in as i32;
        let fs_out = fs_out as i32;
        s.inv_ratio_q16 = ((fs_in << (14 + up2x)) / fs_out) << 2;
        while smulww(s.inv_ratio_q16, fs_out) < fs_in << up2x {
            s.inv_ratio_q16 += 1;
        }
        s
    }

    /// Resample `input` (at least 1 ms) into `out`.
    pub fn process(&mut self, out: &mut [i16], input: &[i16]) {
        let in_len = input.len();
        let n_samples = self.fs_in_khz - self.input_delay;
        self.delay_buf[self.input_delay..self.fs_in_khz].copy_from_slice(&input[..n_samples]);
        let first_in: [i16; 48] = self.delay_buf;
        let fin = self.fs_in_khz;
        let fout = self.fs_out_khz;
        let rest = &input[n_samples..n_samples + in_len - fin];
        match self.kind {
            Kind::Up2 => {
                up2_hq(&mut self.s_iir, out, &first_in[..fin]);
                up2_hq(&mut self.s_iir, &mut out[fout..], rest);
            }
            Kind::IirFir => {
                let o = self.iir_fir(out, &first_in[..fin]);
                debug_assert_eq!(o, fout);
                self.iir_fir(&mut out[fout..], rest);
            }
            Kind::DownFir => {
                self.down_fir(out, &first_in[..fin]);
                self.down_fir(&mut out[fout..], rest);
            }
            Kind::Copy => {
                out[..fin].copy_from_slice(&first_in[..fin]);
                out[fout..fout + in_len - fin].copy_from_slice(rest);
            }
        }
        let d = self.input_delay;
        self.delay_buf[..d].copy_from_slice(&input[in_len - d..]);
    }

    fn iir_fir(&mut self, out: &mut [i16], input: &[i16]) -> usize {
        let mut buf = [0i16; 2 * 16 * MAX_BATCH_MS + ORDER_FIR_12];
        buf[..ORDER_FIR_12].copy_from_slice(&self.s_fir_i16[..ORDER_FIR_12]);
        let inc = self.inv_ratio_q16;
        let mut in_pos = 0;
        let mut remaining = input.len();
        let mut written = 0;
        let mut n_in;
        loop {
            n_in = remaining.min(self.batch_size);
            up2_hq(
                &mut self.s_iir,
                &mut buf[ORDER_FIR_12..],
                &input[in_pos..in_pos + n_in],
            );
            let max_index_q16 = (n_in as i32) << (16 + 1);
            let mut index_q16 = 0i32;
            while index_q16 < max_index_q16 {
                let table_index = smulwb(index_q16 & 0xffff, 12) as usize;
                let at = (index_q16 >> 16) as usize;
                let b: &[i16; 8] = buf[at..at + 8].try_into().expect("8 taps");
                let c0: &[i16; 4] = RESAMPLER_FRAC_FIR_12[table_index * 4..table_index * 4 + 4]
                    .try_into()
                    .expect("phase");
                let c1: &[i16; 4] = RESAMPLER_FRAC_FIR_12
                    [(11 - table_index) * 4..(11 - table_index) * 4 + 4]
                    .try_into()
                    .expect("mirrored phase");
                let mut res = i32::from(b[0]) * i32::from(c0[0]);
                res += i32::from(b[1]) * i32::from(c0[1]);
                res += i32::from(b[2]) * i32::from(c0[2]);
                res += i32::from(b[3]) * i32::from(c0[3]);
                res += i32::from(b[4]) * i32::from(c1[3]);
                res += i32::from(b[5]) * i32::from(c1[2]);
                res += i32::from(b[6]) * i32::from(c1[1]);
                res += i32::from(b[7]) * i32::from(c1[0]);
                out[written] = sat16(rshift_round(res, 15)) as i16;
                written += 1;
                index_q16 += inc;
            }
            in_pos += n_in;
            remaining -= n_in;
            if remaining > 0 {
                buf.copy_within(n_in << 1..(n_in << 1) + ORDER_FIR_12, 0);
            } else {
                break;
            }
        }
        self.s_fir_i16[..ORDER_FIR_12].copy_from_slice(&buf[n_in << 1..(n_in << 1) + ORDER_FIR_12]);
        written
    }

    fn down_fir(&mut self, out: &mut [i16], input: &[i16]) -> usize {
        let order = self.fir_order;
        let mut buf = [0i32; 16 * MAX_BATCH_MS + 36];
        buf[..order].copy_from_slice(&self.s_fir_i32[..order]);
        let coefs = &self.coefs[2..];
        let inc = self.inv_ratio_q16;
        let mut in_pos = 0;
        let mut remaining = input.len() as isize;
        let mut written = 0;
        let mut n_in;
        loop {
            n_in = (remaining as usize).min(self.batch_size);
            // Second-order AR filter, output in Q8.
            let a0 = i32::from(self.coefs[0]);
            let a1 = i32::from(self.coefs[1]);
            for k in 0..n_in {
                let out32 = self.s_iir[0].wrapping_add(i32::from(input[in_pos + k]) << 8);
                buf[order + k] = out32;
                let out32 = out32 << 2;
                self.s_iir[0] = smlawb(self.s_iir[1], out32, a0);
                self.s_iir[1] = smulwb(out32, a1);
            }
            let max_index_q16 = (n_in as i32) << 16;
            let mut index_q16 = 0i32;
            while index_q16 < max_index_q16 {
                let b = &buf[(index_q16 >> 16) as usize..];
                let c = |i: usize| i32::from(coefs[i]);
                let res = match order {
                    18 => {
                        let ii = smulwb(index_q16 & 0xffff, self.fir_fracs) as usize;
                        let p1 = &coefs[9 * ii..];
                        let p2 = &coefs[9 * (self.fir_fracs as usize - 1 - ii)..];
                        let mut r = smulwb(b[0], i32::from(p1[0]));
                        for t in 1..9 {
                            r = smlawb(r, b[t], i32::from(p1[t]));
                        }
                        for t in 0..9 {
                            r = smlawb(r, b[17 - t], i32::from(p2[t]));
                        }
                        r
                    }
                    24 => {
                        let mut r = smulwb(b[0].wrapping_add(b[23]), c(0));
                        for t in 1..12 {
                            r = smlawb(r, b[t].wrapping_add(b[23 - t]), c(t));
                        }
                        r
                    }
                    _ => {
                        let mut r = smulwb(b[0].wrapping_add(b[35]), c(0));
                        for t in 1..18 {
                            r = smlawb(r, b[t].wrapping_add(b[35 - t]), c(t));
                        }
                        r
                    }
                };
                out[written] = sat16(rshift_round(res, 6)) as i16;
                written += 1;
                index_q16 += inc;
            }
            in_pos += n_in;
            remaining -= n_in as isize;
            if remaining > 1 {
                buf.copy_within(n_in..n_in + order, 0);
            } else {
                break;
            }
        }
        self.s_fir_i32[..order].copy_from_slice(&buf[n_in..n_in + order]);
        written
    }
}

fn up2_hq(s: &mut [i32; 6], out: &mut [i16], input: &[i16]) {
    for (k, &x) in input.iter().enumerate() {
        let in32 = i32::from(x) << 10;
        let y = in32.wrapping_sub(s[0]);
        let xx = smulwb(y, UP2_HQ_0[0]);
        let o1 = s[0].wrapping_add(xx);
        s[0] = in32.wrapping_add(xx);
        let y = o1.wrapping_sub(s[1]);
        let xx = smulwb(y, UP2_HQ_0[1]);
        let o2 = s[1].wrapping_add(xx);
        s[1] = o1.wrapping_add(xx);
        let y = o2.wrapping_sub(s[2]);
        let xx = smlawb(y, y, UP2_HQ_0[2]);
        let o1 = s[2].wrapping_add(xx);
        s[2] = o2.wrapping_add(xx);
        out[2 * k] = sat16(rshift_round(o1, 10)) as i16;
        let y = in32.wrapping_sub(s[3]);
        let xx = smulwb(y, UP2_HQ_1[0]);
        let o1 = s[3].wrapping_add(xx);
        s[3] = in32.wrapping_add(xx);
        let y = o1.wrapping_sub(s[4]);
        let xx = smulwb(y, UP2_HQ_1[1]);
        let o2 = s[4].wrapping_add(xx);
        s[4] = o1.wrapping_add(xx);
        let y = o2.wrapping_sub(s[5]);
        let xx = smlawb(y, y, UP2_HQ_1[2]);
        let o1 = s[5].wrapping_add(xx);
        s[5] = o2.wrapping_add(xx);
        out[2 * k + 1] = sat16(rshift_round(o1, 10)) as i16;
    }
}
