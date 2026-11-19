//! CELT layer decoder (RFC 6716 section 4.3), float synthesis.

mod bands;
mod energy;
mod mdct;
mod plc;
mod pvq;
mod rate;
pub mod tables;

use bands::{anti_collapse, denormalise_bands, lcg_rand, quant_all_bands, AllBands};
use mdct::{mdct, MdctScratch};
use plc::LPC_ORDER;
use tables::*;

use crate::range::{RangeDecoder, BITRES};
use crate::Error;

const DECODE_BUFFER_SIZE: usize = 2048;
const MAX_PERIOD: usize = 1024;
const PLC_PITCH_LAG_MAX: usize = 720;
const PLC_PITCH_LAG_MIN: usize = 100;
const COMBFILTER_MINPERIOD: usize = 15;
const VERY_SMALL: f32 = 1e-30;

pub use mdct::Complex;

/// Window shared with the Opus layer cross-fades.
pub fn window() -> &'static [f32; OVERLAP] {
    &mdct().window
}

#[derive(Clone)]
pub struct CeltDecoder {
    channels: usize,
    stream_channels: usize,
    downsample: usize,
    start: usize,
    end: usize,
    disable_inv: bool,

    rng: u32,
    error: bool,
    last_pitch_index: usize,
    loss_duration: u32,
    skip_plc: bool,
    postfilter_period: usize,
    postfilter_period_old: usize,
    postfilter_gain: f32,
    postfilter_gain_old: f32,
    postfilter_tapset: usize,
    postfilter_tapset_old: usize,
    prefilter_and_fold: bool,
    preemph_mem: [f32; 2],
    decode_mem: [Vec<f32>; 2],
    lpc: [[f32; LPC_ORDER]; 2],
    old_band_e: [f32; 2 * NB_EBANDS],
    old_log_e: [f32; 2 * NB_EBANDS],
    old_log_e2: [f32; 2 * NB_EBANDS],
    background_log_e: [f32; 2 * NB_EBANDS],

    scratch: MdctScratch,
    x: Vec<f32>,
    freq: Vec<f32>,
    freq2: Vec<f32>,
}

fn resampling_factor(rate: u32) -> Option<usize> {
    Some(match rate {
        48_000 => 1,
        24_000 => 2,
        16_000 => 3,
        12_000 => 4,
        8_000 => 6,
        _ => return None,
    })
}

impl CeltDecoder {
    pub fn new(rate: u32, channels: usize) -> Result<Self, Error> {
        let downsample = resampling_factor(rate).ok_or(Error::BadArgument)?;
        if !(1..=2).contains(&channels) {
            return Err(Error::BadArgument);
        }
        let n = SHORT_MDCT_SIZE << MAX_LM;
        let mut d = Self {
            channels,
            stream_channels: channels,
            downsample,
            start: 0,
            end: NB_EBANDS,
            disable_inv: channels == 1,
            rng: 0,
            error: false,
            last_pitch_index: 0,
            loss_duration: 0,
            skip_plc: true,
            postfilter_period: 0,
            postfilter_period_old: 0,
            postfilter_gain: 0.0,
            postfilter_gain_old: 0.0,
            postfilter_tapset: 0,
            postfilter_tapset_old: 0,
            prefilter_and_fold: false,
            preemph_mem: [0.0; 2],
            decode_mem: [
                vec![0.0; DECODE_BUFFER_SIZE + OVERLAP],
                vec![0.0; DECODE_BUFFER_SIZE + OVERLAP],
            ],
            lpc: [[0.0; LPC_ORDER]; 2],
            old_band_e: [0.0; 2 * NB_EBANDS],
            old_log_e: [-28.0; 2 * NB_EBANDS],
            old_log_e2: [-28.0; 2 * NB_EBANDS],
            background_log_e: [0.0; 2 * NB_EBANDS],
            scratch: MdctScratch::default(),
            x: vec![0.0; 2 * n],
            freq: vec![0.0; n],
            freq2: vec![0.0; n],
        };
        d.reset();
        Ok(d)
    }

    /// `OPUS_RESET_STATE`: clear everything that evolves while decoding.
    pub fn reset(&mut self) {
        self.rng = 0;
        self.error = false;
        self.last_pitch_index = 0;
        self.loss_duration = 0;
        self.skip_plc = true;
        self.postfilter_period = 0;
        self.postfilter_period_old = 0;
        self.postfilter_gain = 0.0;
        self.postfilter_gain_old = 0.0;
        self.postfilter_tapset = 0;
        self.postfilter_tapset_old = 0;
        self.prefilter_and_fold = false;
        self.preemph_mem = [0.0; 2];
        for m in &mut self.decode_mem {
            m.fill(0.0);
        }
        self.lpc = [[0.0; LPC_ORDER]; 2];
        self.old_band_e = [0.0; 2 * NB_EBANDS];
        self.old_log_e = [-28.0; 2 * NB_EBANDS];
        self.old_log_e2 = [-28.0; 2 * NB_EBANDS];
        self.background_log_e = [0.0; 2 * NB_EBANDS];
    }

    pub fn set_start_band(&mut self, start: usize) {
        self.start = start;
    }

    pub fn set_end_band(&mut self, end: usize) {
        self.end = end;
    }

    pub fn set_stream_channels(&mut self, c: usize) {
        self.stream_channels = c;
    }

    pub fn final_range(&self) -> u32 {
        self.rng
    }

    /// Decode one CELT frame of `frame_size` output samples (at the decoder
    /// rate) into interleaved `pcm` (full scale 1.0). `data` is `None` (or
    /// one byte or less) for concealment. When `dec` is given it continues an
    /// existing range decoder over `data`; otherwise a new one is started.
    pub fn decode(
        &mut self,
        data: Option<&[u8]>,
        dec: Option<&mut RangeDecoder>,
        pcm: &mut [f32],
        frame_size: usize,
    ) -> Result<usize, Error> {
        let frame_size = frame_size * self.downsample;
        let lm = (0..=MAX_LM)
            .find(|&lm| SHORT_MDCT_SIZE << lm == frame_size)
            .ok_or(Error::BadArgument)?;
        let n = SHORT_MDCT_SIZE << lm;
        match data {
            Some(d) if d.len() > 1275 => Err(Error::BadArgument),
            Some(d) if d.len() > 1 => {
                if self.loss_duration == 0 {
                    self.skip_plc = false;
                }
                match dec {
                    Some(dec) => self.decode_frame(d.len(), dec, pcm, lm)?,
                    None => {
                        let mut own = RangeDecoder::new(d);
                        self.decode_frame(d.len(), &mut own, pcm, lm)?;
                    }
                }
                Ok(frame_size / self.downsample)
            }
            _ => {
                self.decode_lost(n, lm);
                self.deemphasis(pcm, n);
                Ok(frame_size / self.downsample)
            }
        }
    }

    fn decode_frame(
        &mut self,
        len: usize,
        dec: &mut RangeDecoder,
        pcm: &mut [f32],
        lm: usize,
    ) -> Result<(), Error> {
        let cc = self.channels;
        let m = 1usize << lm;
        let n = m * SHORT_MDCT_SIZE;
        let start = self.start;
        let end = self.end;
        let eff_end = end.min(NB_EBANDS);
        let c = self.stream_channels;
        if c == 1 {
            for i in 0..NB_EBANDS {
                self.old_band_e[i] = self.old_band_e[i].max(self.old_band_e[NB_EBANDS + i]);
            }
        }
        let total_bits_whole = len as i32 * 8;
        let mut tell = dec.tell();
        let silence = if tell >= total_bits_whole {
            true
        } else if tell == 1 {
            dec.bit_logp(15)
        } else {
            false
        };
        if silence {
            tell = len as i32 * 8;
            dec.skip_to_bits(tell);
        }
        let mut postfilter_gain = 0.0f32;
        let mut postfilter_pitch = 0usize;
        let mut postfilter_tapset = 0usize;
        if start == 0 && tell + 16 <= total_bits_whole {
            if dec.bit_logp(1) {
                let octave = dec.uint(6);
                postfilter_pitch = ((16 << octave) + dec.bits(4 + octave) - 1) as usize;
                let qg = dec.bits(3);
                if dec.tell() + 2 <= total_bits_whole {
                    postfilter_tapset = dec.icdf(&TAPSET_ICDF, 2);
                }
                postfilter_gain = 0.09375 * (qg + 1) as f32;
            }
            tell = dec.tell();
        }
        let is_transient = if lm > 0 && tell + 3 <= total_bits_whole {
            let t = dec.bit_logp(3);
            tell = dec.tell();
            t
        } else {
            false
        };
        let intra_ener = if tell + 3 <= total_bits_whole {
            dec.bit_logp(3)
        } else {
            false
        };
        if !intra_ener && self.loss_duration != 0 {
            let missing = 10.min(self.loss_duration >> lm) as f32;
            let safety = match lm {
                0 => 1.5,
                1 => 0.5,
                _ => 0.0,
            };
            for ch in 0..2 {
                for i in start..end {
                    let k = ch * NB_EBANDS + i;
                    if self.old_band_e[k] < self.old_log_e[k].max(self.old_log_e2[k]) {
                        let e0 = self.old_band_e[k];
                        let e1 = self.old_log_e[k];
                        let e2 = self.old_log_e2[k];
                        let slope = (e1 - e0).max(0.5 * (e2 - e0));
                        let e0 = e0 - (0.0f32).max((1.0 + missing) * slope);
                        self.old_band_e[k] = (-20.0f32).max(e0);
                    } else {
                        self.old_band_e[k] = self.old_band_e[k]
                            .min(self.old_log_e[k])
                            .min(self.old_log_e2[k]);
                    }
                    self.old_band_e[k] -= safety;
                }
            }
        }
        energy::unquant_coarse(start, end, &mut self.old_band_e, intra_ener, dec, c, lm);

        let mut tf_res = [0i32; NB_EBANDS];
        tf_decode(start, end, is_transient, &mut tf_res, lm, dec);

        tell = dec.tell();
        let spread = if tell + 4 <= total_bits_whole {
            dec.icdf(&SPREAD_ICDF, 5)
        } else {
            2
        };
        let mut cap = [0i32; NB_EBANDS];
        rate::init_caps(&mut cap, lm, c);
        let mut offsets = [0i32; NB_EBANDS];
        let mut dynalloc_logp = 6;
        let mut total_bits = total_bits_whole << BITRES;
        let mut tellf = dec.tell_frac() as i32;
        for i in start..end {
            let width = (c as i32 * i32::from(EBANDS[i + 1] - EBANDS[i])) << lm;
            let quanta = (width << BITRES).min((6 << BITRES).max(width));
            let mut loop_logp = dynalloc_logp;
            let mut boost = 0;
            while tellf + (loop_logp << BITRES) < total_bits && boost < cap[i] {
                let flag = dec.bit_logp(loop_logp as u32);
                tellf = dec.tell_frac() as i32;
                if !flag {
                    break;
                }
                boost += quanta;
                total_bits -= quanta;
                loop_logp = 1;
            }
            offsets[i] = boost;
            if boost > 0 {
                dynalloc_logp = 2.max(dynalloc_logp - 1);
            }
        }
        let alloc_trim = if tellf + (6 << BITRES) <= total_bits {
            dec.icdf(&TRIM_ICDF, 7) as i32
        } else {
            5
        };
        let mut bits = ((len as i32 * 8) << BITRES) - dec.tell_frac() as i32 - 1;
        let anti_collapse_rsv = if is_transient && lm >= 2 && bits >= ((lm as i32 + 2) << BITRES) {
            1 << BITRES
        } else {
            0
        };
        bits -= anti_collapse_rsv;
        let alloc =
            rate::compute_allocation(start, end, &offsets, &cap, alloc_trim, bits, c, lm, dec);
        energy::unquant_fine(start, end, &mut self.old_band_e, &alloc.fine_quant, dec, c);
        for ch in 0..cc {
            self.decode_mem[ch].copy_within(n.., 0);
        }

        let mut collapse_masks = [0u8; 2 * NB_EBANDS];
        let x = &mut self.x[..c * n];
        let (xs, ys) = x.split_at_mut(n);
        let params = AllBands {
            start,
            end,
            pulses: &alloc.pulses,
            short_blocks: is_transient,
            spread,
            dual_stereo: alloc.dual_stereo,
            intensity: alloc.intensity,
            tf_res: &tf_res,
            total_bits: len as i32 * (8 << BITRES) - anti_collapse_rsv,
            balance: alloc.balance,
            lm,
            coded_bands: alloc.coded_bands,
            disable_inv: self.disable_inv,
        };
        quant_all_bands(
            &params,
            xs,
            if c == 2 { Some(ys) } else { None },
            &mut collapse_masks,
            dec,
            &mut self.rng,
        );
        let anti_collapse_on = anti_collapse_rsv > 0 && dec.bits(1) != 0;
        let bits_left = len as i32 * 8 - dec.tell();
        energy::unquant_finalise(
            start,
            end,
            &mut self.old_band_e,
            &alloc.fine_quant,
            &alloc.fine_priority,
            bits_left,
            dec,
            c,
        );
        if anti_collapse_on {
            anti_collapse(
                &mut self.x[..c * n],
                &collapse_masks,
                lm,
                c,
                n,
                start,
                end,
                &self.old_band_e,
                &self.old_log_e,
                &self.old_log_e2,
                &alloc.pulses,
                self.rng,
            );
        }
        if silence {
            self.old_band_e[..c * NB_EBANDS].fill(-28.0);
        }
        if self.prefilter_and_fold {
            self.prefilter_and_fold(n);
        }
        self.synthesis(c, start, eff_end, is_transient, lm, silence);

        let win = &mdct().window;
        for ch in 0..cc {
            self.postfilter_period = self.postfilter_period.max(COMBFILTER_MINPERIOD);
            self.postfilter_period_old = self.postfilter_period_old.max(COMBFILTER_MINPERIOD);
            let buf = &mut self.decode_mem[ch];
            let off = DECODE_BUFFER_SIZE - n;
            comb_filter_inplace(
                buf,
                off,
                self.postfilter_period_old,
                self.postfilter_period,
                SHORT_MDCT_SIZE,
                self.postfilter_gain_old,
                self.postfilter_gain,
                self.postfilter_tapset_old,
                self.postfilter_tapset,
                win,
            );
            if lm != 0 {
                comb_filter_inplace(
                    buf,
                    off + SHORT_MDCT_SIZE,
                    self.postfilter_period,
                    postfilter_pitch,
                    n - SHORT_MDCT_SIZE,
                    self.postfilter_gain,
                    postfilter_gain,
                    self.postfilter_tapset,
                    postfilter_tapset,
                    win,
                );
            }
        }
        self.postfilter_period_old = self.postfilter_period;
        self.postfilter_gain_old = self.postfilter_gain;
        self.postfilter_tapset_old = self.postfilter_tapset;
        self.postfilter_period = postfilter_pitch;
        self.postfilter_gain = postfilter_gain;
        self.postfilter_tapset = postfilter_tapset;
        if lm != 0 {
            self.postfilter_period_old = self.postfilter_period;
            self.postfilter_gain_old = self.postfilter_gain;
            self.postfilter_tapset_old = self.postfilter_tapset;
        }
        if c == 1 {
            let (a, b) = self.old_band_e.split_at_mut(NB_EBANDS);
            b.copy_from_slice(a);
        }
        if !is_transient {
            self.old_log_e2 = self.old_log_e;
            self.old_log_e = self.old_band_e;
        } else {
            for i in 0..2 * NB_EBANDS {
                self.old_log_e[i] = self.old_log_e[i].min(self.old_band_e[i]);
            }
        }
        let max_background_increase = (160.min(self.loss_duration + m as u32)) as f32 * 0.001;
        for i in 0..2 * NB_EBANDS {
            self.background_log_e[i] =
                (self.background_log_e[i] + max_background_increase).min(self.old_band_e[i]);
        }
        for ch in 0..2 {
            for i in (0..start).chain(end..NB_EBANDS) {
                let k = ch * NB_EBANDS + i;
                self.old_band_e[k] = 0.0;
                self.old_log_e[k] = -28.0;
                self.old_log_e2[k] = -28.0;
            }
        }
        self.rng = dec.range();
        self.deemphasis(pcm, n);
        self.loss_duration = 0;
        self.prefilter_and_fold = false;
        if dec.tell() > 8 * len as i32 {
            return Err(Error::InternalError);
        }
        if dec.error() {
            self.error = true;
        }
        Ok(())
    }

    fn synthesis(
        &mut self,
        c: usize,
        start: usize,
        eff_end: usize,
        is_transient: bool,
        lm: usize,
        silence: bool,
    ) {
        let cc = self.channels;
        let m = 1usize << lm;
        let n = m * SHORT_MDCT_SIZE;
        let (bcount, nb, shift) = if is_transient {
            (m, SHORT_MDCT_SIZE, MAX_LM)
        } else {
            (1, n, MAX_LM - lm)
        };
        let md = mdct();
        let out_off = DECODE_BUFFER_SIZE - n;
        let ds = self.downsample;
        if cc == 2 && c == 1 {
            denormalise_bands(
                &self.x[..n],
                &mut self.freq,
                &self.old_band_e[..NB_EBANDS],
                start,
                eff_end,
                m,
                ds,
                silence,
            );
            for ch in 0..2 {
                for b in 0..bcount {
                    let out = &mut self.decode_mem[ch][out_off + nb * b..];
                    md.backward(&self.freq[b..], out, shift, bcount, &mut self.scratch);
                }
            }
        } else if cc == 1 && c == 2 {
            denormalise_bands(
                &self.x[..n],
                &mut self.freq,
                &self.old_band_e[..NB_EBANDS],
                start,
                eff_end,
                m,
                ds,
                silence,
            );
            denormalise_bands(
                &self.x[n..2 * n],
                &mut self.freq2,
                &self.old_band_e[NB_EBANDS..],
                start,
                eff_end,
                m,
                ds,
                silence,
            );
            for i in 0..n {
                self.freq[i] = 0.5 * self.freq[i] + 0.5 * self.freq2[i];
            }
            for b in 0..bcount {
                let out = &mut self.decode_mem[0][out_off + nb * b..];
                md.backward(&self.freq[b..], out, shift, bcount, &mut self.scratch);
            }
        } else {
            for ch in 0..cc {
                denormalise_bands(
                    &self.x[ch * n..(ch + 1) * n],
                    &mut self.freq,
                    &self.old_band_e[ch * NB_EBANDS..],
                    start,
                    eff_end,
                    m,
                    ds,
                    silence,
                );
                for b in 0..bcount {
                    let out = &mut self.decode_mem[ch][out_off + nb * b..];
                    md.backward(&self.freq[b..], out, shift, bcount, &mut self.scratch);
                }
            }
        }
    }

    fn deemphasis(&mut self, pcm: &mut [f32], n: usize) {
        let cc = self.channels;
        let ds = self.downsample;
        let coef = PREEMPHASIS;
        let scale = 1.0 / 32768.0;
        if cc == 2 && ds == 1 {
            // Both channels in one loop: two independent recursions overlap.
            let x0 = &self.decode_mem[0][DECODE_BUFFER_SIZE - n..DECODE_BUFFER_SIZE];
            let x1 = &self.decode_mem[1][DECODE_BUFFER_SIZE - n..DECODE_BUFFER_SIZE];
            let [mut m0, mut m1] = self.preemph_mem;
            for ((out, &a), &b) in pcm[..2 * n].chunks_exact_mut(2).zip(x0).zip(x1) {
                let t0 = a + VERY_SMALL + m0;
                let t1 = b + VERY_SMALL + m1;
                m0 = coef * t0;
                m1 = coef * t1;
                out[0] = t0 * scale;
                out[1] = t1 * scale;
            }
            self.preemph_mem = [m0, m1];
            return;
        }
        for ch in 0..cc {
            let x = &self.decode_mem[ch][DECODE_BUFFER_SIZE - n..DECODE_BUFFER_SIZE];
            let mut mem = self.preemph_mem[ch];
            if ds == 1 {
                for (j, &v) in x.iter().enumerate() {
                    let tmp = v + VERY_SMALL + mem;
                    mem = coef * tmp;
                    pcm[j * cc + ch] = tmp * scale;
                }
            } else {
                for (j, &v) in x.iter().enumerate() {
                    let tmp = v + VERY_SMALL + mem;
                    mem = coef * tmp;
                    if j % ds == 0 {
                        pcm[(j / ds) * cc + ch] = tmp * scale;
                    }
                }
            }
            self.preemph_mem[ch] = mem;
        }
    }

    fn prefilter_and_fold(&mut self, n: usize) {
        let win = &mdct().window;
        for ch in 0..self.channels {
            let mut etmp = [0f32; OVERLAP];
            comb_filter_oop(
                &mut etmp,
                &self.decode_mem[ch],
                DECODE_BUFFER_SIZE - n,
                self.postfilter_period_old,
                self.postfilter_period,
                OVERLAP,
                -self.postfilter_gain_old,
                -self.postfilter_gain,
                self.postfilter_tapset_old,
                self.postfilter_tapset,
            );
            let buf = &mut self.decode_mem[ch];
            for i in 0..OVERLAP / 2 {
                buf[DECODE_BUFFER_SIZE - n + i] =
                    win[i] * etmp[OVERLAP - 1 - i] + win[OVERLAP - i - 1] * etmp[i];
            }
        }
    }

    fn decode_lost(&mut self, n: usize, lm: usize) {
        let c = self.channels;
        let loss_duration = self.loss_duration;
        let start = self.start;
        let noise_based = loss_duration >= 40 || start != 0 || self.skip_plc;
        if noise_based {
            let end = self.end;
            let eff_end = start.max(end.min(NB_EBANDS));
            for ch in 0..c {
                self.decode_mem[ch].copy_within(n.., 0);
            }
            if self.prefilter_and_fold {
                self.prefilter_and_fold(n);
            }
            let decay = if loss_duration == 0 { 1.5 } else { 0.5 };
            for ch in 0..c {
                for i in start..end {
                    let k = ch * NB_EBANDS + i;
                    self.old_band_e[k] = self.background_log_e[k].max(self.old_band_e[k] - decay);
                }
            }
            let mut seed = self.rng;
            for ch in 0..c {
                for i in start..eff_end {
                    let boffs = n * ch + ((EBANDS[i] as usize) << lm);
                    let blen = ((EBANDS[i + 1] - EBANDS[i]) as usize) << lm;
                    for j in 0..blen {
                        seed = lcg_rand(seed);
                        self.x[boffs + j] = ((seed as i32) >> 20) as f32;
                    }
                    pvq::renormalise_vector(&mut self.x[boffs..boffs + blen], 1.0);
                }
            }
            self.rng = seed;
            self.synthesis(c, start, eff_end, false, lm, false);
            self.prefilter_and_fold = false;
            self.skip_plc = true;
        } else {
            let win = &mdct().window;
            let (pitch_index, fade) = if loss_duration == 0 {
                let p = self.plc_pitch_search();
                self.last_pitch_index = p;
                (p, 1.0f32)
            } else {
                (self.last_pitch_index, 0.8f32)
            };
            let exc_length = (2 * pitch_index).min(MAX_PERIOD);
            let mut exc_buf = vec![0f32; MAX_PERIOD + LPC_ORDER];
            let mut fir_tmp = vec![0f32; exc_length];
            for ch in 0..c {
                let buf = &mut self.decode_mem[ch];
                for i in 0..MAX_PERIOD + LPC_ORDER {
                    exc_buf[i] = buf[DECODE_BUFFER_SIZE - MAX_PERIOD - LPC_ORDER + i];
                }
                if loss_duration == 0 {
                    let mut ac = [0f32; LPC_ORDER + 1];
                    plc::autocorr(
                        &exc_buf[LPC_ORDER..],
                        &mut ac,
                        Some(win),
                        OVERLAP,
                        LPC_ORDER,
                    );
                    ac[0] *= 1.0001;
                    for (i, a) in ac.iter_mut().enumerate().skip(1) {
                        *a -= *a * (0.008 * 0.008) * (i * i) as f32;
                    }
                    plc::lpc(&mut self.lpc[ch], &ac);
                }
                // exc = exc_buf[LPC_ORDER..]
                plc::fir(
                    &exc_buf,
                    LPC_ORDER + MAX_PERIOD - exc_length,
                    &self.lpc[ch],
                    &mut fir_tmp,
                );
                exc_buf[LPC_ORDER + MAX_PERIOD - exc_length..LPC_ORDER + MAX_PERIOD]
                    .copy_from_slice(&fir_tmp);
                let exc = &exc_buf[LPC_ORDER..];
                let decay = {
                    let mut e1 = 1f32;
                    let mut e2 = 1f32;
                    let dl = exc_length >> 1;
                    for i in 0..dl {
                        let e = exc[MAX_PERIOD - dl + i];
                        e1 += e * e;
                        let e = exc[MAX_PERIOD - 2 * dl + i];
                        e2 += e * e;
                    }
                    e1 = e1.min(e2);
                    (e1 / e2).sqrt()
                };
                buf.copy_within(n..DECODE_BUFFER_SIZE, 0);
                let extrapolation_offset = MAX_PERIOD - pitch_index;
                let extrapolation_len = n + OVERLAP;
                let mut attenuation = fade * decay;
                let mut s1 = 0f32;
                let mut j = 0usize;
                for i in 0..extrapolation_len {
                    if j >= pitch_index {
                        j -= pitch_index;
                        attenuation *= decay;
                    }
                    buf[DECODE_BUFFER_SIZE - n + i] = attenuation * exc[extrapolation_offset + j];
                    let tmp = buf[DECODE_BUFFER_SIZE - MAX_PERIOD - n + extrapolation_offset + j];
                    s1 += tmp * tmp;
                    j += 1;
                }
                let mut lpc_mem = [0f32; LPC_ORDER];
                for (i, v) in lpc_mem.iter_mut().enumerate() {
                    *v = buf[DECODE_BUFFER_SIZE - n - 1 - i];
                }
                plc::iir(
                    &mut buf[DECODE_BUFFER_SIZE - n..DECODE_BUFFER_SIZE - n + extrapolation_len],
                    &self.lpc[ch],
                    &mut lpc_mem,
                );
                let mut s2 = 0f32;
                for i in 0..extrapolation_len {
                    let tmp = buf[DECODE_BUFFER_SIZE - n + i];
                    s2 += tmp * tmp;
                }
                #[allow(clippy::neg_cmp_op_on_partial_ord)]
                if !(s1 > 0.2 * s2) {
                    buf[DECODE_BUFFER_SIZE - n..DECODE_BUFFER_SIZE - n + extrapolation_len]
                        .fill(0.0);
                } else if s1 < s2 {
                    let ratio = ((s1 + 1.0) / (s2 + 1.0)).sqrt();
                    for i in 0..OVERLAP {
                        let g = 1.0 - win[i] * (1.0 - ratio);
                        buf[DECODE_BUFFER_SIZE - n + i] *= g;
                    }
                    for i in OVERLAP..extrapolation_len {
                        buf[DECODE_BUFFER_SIZE - n + i] *= ratio;
                    }
                }
            }
            self.prefilter_and_fold = true;
        }
        self.loss_duration = 10_000.min(loss_duration + (1 << lm));
    }

    fn plc_pitch_search(&self) -> usize {
        let mut lp = vec![0f32; DECODE_BUFFER_SIZE >> 1];
        let chans: Vec<&[f32]> = self.decode_mem[..self.channels]
            .iter()
            .map(|v| &v[..])
            .collect();
        plc::pitch_downsample(&chans, &mut lp, DECODE_BUFFER_SIZE);
        let p = plc::pitch_search(
            &lp[PLC_PITCH_LAG_MAX >> 1..],
            &lp,
            DECODE_BUFFER_SIZE - PLC_PITCH_LAG_MAX,
            PLC_PITCH_LAG_MAX - PLC_PITCH_LAG_MIN,
        );
        PLC_PITCH_LAG_MAX - p
    }
}

fn tf_decode(
    start: usize,
    end: usize,
    is_transient: bool,
    tf_res: &mut [i32; NB_EBANDS],
    lm: usize,
    dec: &mut RangeDecoder,
) {
    let mut budget = dec.storage() as u32 * 8;
    let mut tell = dec.tell() as u32;
    let mut logp = if is_transient { 2 } else { 4 };
    let tf_select_rsv = lm > 0 && tell + logp + 1 <= budget;
    budget -= u32::from(tf_select_rsv);
    let mut tf_changed = 0;
    let mut curr = 0;
    for r in tf_res.iter_mut().take(end).skip(start) {
        if tell + logp <= budget {
            curr ^= i32::from(dec.bit_logp(logp));
            tell = dec.tell() as u32;
            tf_changed |= curr;
        }
        *r = curr;
        logp = if is_transient { 4 } else { 5 };
    }
    let t = 4 * usize::from(is_transient);
    let mut tf_select = 0;
    if tf_select_rsv
        && TF_SELECT_TABLE[lm][t + tf_changed as usize]
            != TF_SELECT_TABLE[lm][t + 2 + tf_changed as usize]
    {
        tf_select = usize::from(dec.bit_logp(1));
    }
    for r in tf_res.iter_mut().take(end).skip(start) {
        *r = i32::from(TF_SELECT_TABLE[lm][t + 2 * tf_select + *r as usize]);
    }
}

/// In-place pitch post-filter over `buf[off..off + n]`, cross-fading from
/// (`t0`, `g0`, `tapset0`) to (`t1`, `g1`, `tapset1`) over the overlap.
/// The newest four history taps are carried in registers so each output
/// sample loads one history value (the filter is recursive in place, which
/// is what the reference computes too).
#[allow(clippy::too_many_arguments)]
fn comb_filter_inplace(
    buf: &mut [f32],
    off: usize,
    t0: usize,
    t1: usize,
    n: usize,
    g0: f32,
    g1: f32,
    tapset0: usize,
    tapset1: usize,
    window: &[f32; OVERLAP],
) {
    if g0 == 0.0 && g1 == 0.0 {
        return;
    }
    let t0 = t0.max(COMBFILTER_MINPERIOD);
    let t1 = t1.max(COMBFILTER_MINPERIOD);
    let g00 = g0 * COMB_GAINS[tapset0][0];
    let g01 = g0 * COMB_GAINS[tapset0][1];
    let g02 = g0 * COMB_GAINS[tapset0][2];
    let g10 = g1 * COMB_GAINS[tapset1][0];
    let g11 = g1 * COMB_GAINS[tapset1][1];
    let g12 = g1 * COMB_GAINS[tapset1][2];
    let overlap = if g0 == g1 && t0 == t1 && tapset0 == tapset1 {
        0
    } else {
        OVERLAP.min(n)
    };
    let mut x1 = buf[off + 1 - t1];
    let mut x2 = buf[off - t1];
    let mut x3 = buf[off - t1 - 1];
    let mut x4 = buf[off - t1 - 2];
    for (i, &w) in window[..overlap].iter().enumerate() {
        let p = off + i;
        let f = w * w;
        let x0 = buf[p + 2 - t1];
        let old: &[f32; 5] = buf[p - t0 - 2..=p - t0 + 2].try_into().expect("5 taps");
        let y = buf[p]
            + ((1.0 - f) * g00) * old[2]
            + ((1.0 - f) * g01) * (old[3] + old[1])
            + ((1.0 - f) * g02) * (old[4] + old[0])
            + (f * g10) * x2
            + (f * g11) * (x1 + x3)
            + (f * g12) * (x0 + x4);
        buf[p] = y;
        x4 = x3;
        x3 = x2;
        x2 = x1;
        x1 = x0;
    }
    if g1 == 0.0 {
        return;
    }
    for p in off + overlap..off + n {
        let x0 = buf[p + 2 - t1];
        buf[p] = buf[p] + g10 * x2 + g11 * (x1 + x3) + g12 * (x0 + x4);
        x4 = x3;
        x3 = x2;
        x2 = x1;
        x1 = x0;
    }
}

/// Out-of-place constant post-filter (the reference calls it with no
/// overlap window): `y[i] = x[off + i] + taps(x, t1)`.
#[allow(clippy::too_many_arguments)]
fn comb_filter_oop(
    y: &mut [f32],
    x: &[f32],
    off: usize,
    _t0: usize,
    t1: usize,
    n: usize,
    g0: f32,
    g1: f32,
    _tapset0: usize,
    tapset1: usize,
) {
    if (g0 == 0.0 && g1 == 0.0) || g1 == 0.0 {
        y[..n].copy_from_slice(&x[off..off + n]);
        return;
    }
    let t1 = t1.max(COMBFILTER_MINPERIOD);
    let g10 = g1 * COMB_GAINS[tapset1][0];
    let g11 = g1 * COMB_GAINS[tapset1][1];
    let g12 = g1 * COMB_GAINS[tapset1][2];
    for i in 0..n {
        let p = off + i;
        y[i] = x[p]
            + g10 * x[p - t1]
            + g11 * (x[p + 1 - t1] + x[p - t1 - 1])
            + g12 * (x[p + 2 - t1] + x[p - t1 - 2]);
    }
}
