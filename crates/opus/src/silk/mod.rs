//! SILK layer decoder (RFC 6716 section 4.2), fixed point.

mod channel;
mod lpc;
mod macros;
mod resampler;
mod tables;

use channel::*;
use macros::*;
use tables::*;

use crate::range::RangeDecoder;
use crate::Error;

/// Parameters from the Opus layer (`silk_DecControlStruct`).
#[derive(Clone, Copy, Debug)]
pub struct SilkControl {
    pub channels_api: usize,
    pub channels_internal: usize,
    pub api_rate: u32,
    pub internal_rate: u32,
    /// Opus frame duration in ms (10, 20, 40 or 60).
    pub payload_ms: usize,
}

#[derive(Clone, Default)]
struct StereoState {
    pred_prev_q13: [i32; 2],
    s_mid: [i16; 2],
    s_side: [i16; 2],
}

#[derive(Clone)]
pub struct SilkDecoder {
    ch: [ChannelState; 2],
    stereo: StereoState,
    n_channels_api: usize,
    n_channels_internal: usize,
    prev_decode_only_middle: bool,
    tmp: [Vec<i16>; 2],
    resample_out: Vec<i16>,
}

impl Default for SilkDecoder {
    fn default() -> Self {
        Self::new()
    }
}

fn stereo_decode_pred(dec: &mut RangeDecoder) -> [i32; 2] {
    let n = dec.icdf(&STEREO_PRED_JOINT_ICDF, 8);
    let mut ix = [[0usize; 3]; 2];
    ix[0][2] = n / 5;
    ix[1][2] = n - 5 * ix[0][2];
    for row in &mut ix {
        row[0] = dec.icdf(&UNIFORM3_ICDF, 8);
        row[1] = dec.icdf(&UNIFORM5_ICDF, 8);
    }
    let mut pred = [0i32; 2];
    for n in 0..2 {
        ix[n][0] += 3 * ix[n][2];
        let low = i32::from(STEREO_PRED_QUANT_Q13[ix[n][0]]);
        let step = smulwb(i32::from(STEREO_PRED_QUANT_Q13[ix[n][0] + 1]) - low, 6554); // 0.5/5 in Q16
        pred[n] = smlabb(low, step, 2 * ix[n][1] as i32 + 1);
    }
    pred[0] -= pred[1];
    pred
}

impl SilkDecoder {
    pub fn new() -> Self {
        Self {
            ch: [ChannelState::new(), ChannelState::new()],
            stereo: StereoState::default(),
            n_channels_api: 0,
            n_channels_internal: 0,
            prev_decode_only_middle: false,
            tmp: [vec![0; MAX_FRAME_LENGTH + 2], vec![0; MAX_FRAME_LENGTH + 2]],
            resample_out: vec![0; 960],
        }
    }

    /// `silk_ResetDecoder`.
    pub fn reset(&mut self) {
        for c in &mut self.ch {
            c.reset();
        }
        self.stereo = StereoState::default();
        self.prev_decode_only_middle = false;
    }

    /// Decode one SILK frame (10 or 20 ms) into interleaved `out` at the API
    /// rate. `lost_flag`: 0 normal, 1 lost (conceal), 2 FEC (LBRR).
    pub fn decode(
        &mut self,
        ctl: &SilkControl,
        lost_flag: u8,
        new_packet: bool,
        mut dec: Option<&mut RangeDecoder>,
        out: &mut [i16],
    ) -> Result<usize, Error> {
        let nci = ctl.channels_internal;
        let nca = ctl.channels_api;
        if new_packet {
            for c in &mut self.ch[..nci] {
                c.n_frames_decoded = 0;
            }
        }
        if nci > self.n_channels_internal {
            self.ch[1] = ChannelState::new();
        }
        let stereo_to_mono = nci == 1
            && self.n_channels_internal == 2
            && ctl.internal_rate == 1000 * self.ch[0].fs_khz as u32;
        if self.ch[0].n_frames_decoded == 0 {
            for c in &mut self.ch[..nci] {
                let (frames, subfr) = match ctl.payload_ms {
                    0 | 10 => (1, 2),
                    20 => (1, 4),
                    40 => (2, 4),
                    60 => (3, 4),
                    _ => return Err(Error::BadArgument),
                };
                c.n_frames_per_packet = frames;
                c.nb_subfr = subfr;
                let fs_khz = (ctl.internal_rate >> 10) as usize + 1;
                if !matches!(fs_khz, 8 | 12 | 16) {
                    return Err(Error::BadArgument);
                }
                c.set_fs(fs_khz, ctl.api_rate);
            }
        }
        if nca == 2 && nci == 2 && (self.n_channels_api == 1 || self.n_channels_internal == 1) {
            self.stereo.pred_prev_q13 = [0; 2];
            self.stereo.s_side = [0; 2];
            self.ch[1].resampler = self.ch[0].resampler.clone();
        }
        self.n_channels_api = nca;
        self.n_channels_internal = nci;

        let mut ms_pred_q13 = [0i32; 2];
        let mut decode_only_middle = false;
        if lost_flag != 1 && self.ch[0].n_frames_decoded == 0 {
            let d = dec.as_deref_mut().ok_or(Error::InternalError)?;
            for c in &mut self.ch[..nci] {
                for i in 0..c.n_frames_per_packet {
                    c.vad_flags[i] = d.bit_logp(1);
                }
                c.lbrr_flag = d.bit_logp(1);
            }
            for c in &mut self.ch[..nci] {
                c.lbrr_flags = [false; MAX_FRAMES_PER_PACKET];
                if c.lbrr_flag {
                    if c.n_frames_per_packet == 1 {
                        c.lbrr_flags[0] = true;
                    } else {
                        let icdf: &[u8] = if c.n_frames_per_packet == 2 {
                            &LBRR_FLAGS_2_ICDF
                        } else {
                            &LBRR_FLAGS_3_ICDF
                        };
                        let sym = d.icdf(icdf, 8) + 1;
                        for i in 0..c.n_frames_per_packet {
                            c.lbrr_flags[i] = (sym >> i) & 1 != 0;
                        }
                    }
                }
            }
            if lost_flag == 0 {
                // Skip the LBRR data.
                for i in 0..self.ch[0].n_frames_per_packet {
                    for n in 0..nci {
                        if self.ch[n].lbrr_flags[i] {
                            if nci == 2 && n == 0 {
                                stereo_decode_pred(d);
                                if !self.ch[1].lbrr_flags[i] {
                                    d.icdf(&STEREO_ONLY_CODE_MID_ICDF, 8);
                                }
                            }
                            let cond = if i > 0 && self.ch[n].lbrr_flags[i - 1] {
                                CODE_CONDITIONALLY
                            } else {
                                CODE_INDEPENDENTLY
                            };
                            let c = &mut self.ch[n];
                            c.decode_indices(d, i, true, cond);
                            let mut pulses = [0i16; MAX_FRAME_LENGTH];
                            decode_pulses(
                                d,
                                &mut pulses,
                                i32::from(c.indices.signal_type),
                                i32::from(c.indices.quant_offset_type),
                                c.frame_length,
                            );
                        }
                    }
                }
            }
        }
        if nci == 2 {
            let fd = self.ch[0].n_frames_decoded;
            if lost_flag == 0 || (lost_flag == 2 && self.ch[0].lbrr_flags[fd]) {
                let d = dec.as_deref_mut().ok_or(Error::InternalError)?;
                ms_pred_q13 = stereo_decode_pred(d);
                if (lost_flag == 0 && !self.ch[1].vad_flags[fd]) || (lost_flag == 2 && !self.ch[1].lbrr_flags[fd]) {
                    decode_only_middle = d.icdf(&STEREO_ONLY_CODE_MID_ICDF, 8) != 0;
                } else {
                    decode_only_middle = false;
                }
            } else {
                ms_pred_q13 = self.stereo.pred_prev_q13;
            }
        }
        if nci == 2 && !decode_only_middle && self.prev_decode_only_middle {
            let c = &mut self.ch[1];
            c.out_buf.fill(0);
            c.slpc_q14_buf.fill(0);
            c.lag_prev = 100;
            c.last_gain_index = 10;
            c.prev_signal_type = TYPE_NO_VOICE_ACTIVITY;
            c.first_frame_after_reset = true;
        }
        let has_side = if lost_flag == 0 {
            !decode_only_middle
        } else {
            !self.prev_decode_only_middle
                || (nci == 2 && lost_flag == 2 && self.ch[1].lbrr_flags[self.ch[1].n_frames_decoded])
        };
        let mut n_out_dec = 0;
        for n in 0..nci {
            if n == 0 || has_side {
                let frame_index = self.ch[0].n_frames_decoded as isize - n as isize;
                let cond = if frame_index <= 0 {
                    CODE_INDEPENDENTLY
                } else if lost_flag == 2 {
                    if self.ch[n].lbrr_flags[frame_index as usize - 1] {
                        CODE_CONDITIONALLY
                    } else {
                        CODE_INDEPENDENTLY
                    }
                } else if n > 0 && self.prev_decode_only_middle {
                    CODE_INDEPENDENTLY_NO_LTP_SCALING
                } else {
                    CODE_CONDITIONALLY
                };
                let tmp = &mut self.tmp[n];
                n_out_dec = self.ch[n].decode_frame(dec.as_deref_mut(), &mut tmp[2..], lost_flag, cond);
            } else {
                let len = n_out_dec;
                self.tmp[n][2..2 + len].fill(0);
            }
            self.ch[n].n_frames_decoded += 1;
        }
        let fs_khz = self.ch[0].fs_khz;
        if nca == 2 && nci == 2 {
            let (a, b) = self.tmp.split_at_mut(1);
            stereo_ms_to_lr(&mut self.stereo, &mut a[0], &mut b[0], &ms_pred_q13, fs_khz, n_out_dec);
        } else {
            let t = &mut self.tmp[0];
            t[..2].copy_from_slice(&self.stereo.s_mid);
            self.stereo.s_mid.copy_from_slice(&t[n_out_dec..n_out_dec + 2]);
        }
        let n_out = n_out_dec * ctl.api_rate as usize / (fs_khz * 1000);
        if out.len() < n_out * nca {
            return Err(Error::BufferTooSmall);
        }
        for n in 0..nca.min(nci) {
            let input = &self.tmp[n][1..1 + n_out_dec];
            if nca == 2 {
                let ro = &mut self.resample_out[..n_out];
                self.ch[n].resampler.process(ro, input);
                for i in 0..n_out {
                    out[n + 2 * i] = ro[i];
                }
            } else {
                self.ch[n].resampler.process(&mut out[..n_out], input);
            }
        }
        if nca == 2 && nci == 1 {
            if stereo_to_mono {
                let input = &self.tmp[0][1..1 + n_out_dec];
                let ro = &mut self.resample_out[..n_out];
                self.ch[1].resampler.process(ro, input);
                for i in 0..n_out {
                    out[1 + 2 * i] = ro[i];
                }
            } else {
                for i in 0..n_out {
                    out[1 + 2 * i] = out[2 * i];
                }
            }
        }
        if lost_flag == 1 {
            for c in &mut self.ch[..self.n_channels_internal] {
                c.last_gain_index = 10;
            }
        } else {
            self.prev_decode_only_middle = decode_only_middle;
        }
        Ok(n_out)
    }
}

fn stereo_ms_to_lr(st: &mut StereoState, x1: &mut [i16], x2: &mut [i16], pred_q13: &[i32; 2], fs_khz: usize, frame_length: usize) {
    x1[..2].copy_from_slice(&st.s_mid);
    x2[..2].copy_from_slice(&st.s_side);
    st.s_mid.copy_from_slice(&x1[frame_length..frame_length + 2]);
    st.s_side.copy_from_slice(&x2[frame_length..frame_length + 2]);
    let mut pred0 = st.pred_prev_q13[0];
    let mut pred1 = st.pred_prev_q13[1];
    let interp = 8 * fs_khz;
    let denom_q16 = (1i32 << 16) / interp as i32;
    let delta0 = rshift_round(smulbb(pred_q13[0] - st.pred_prev_q13[0], denom_q16), 16);
    let delta1 = rshift_round(smulbb(pred_q13[1] - st.pred_prev_q13[1], denom_q16), 16);
    let mix = |x1: &[i16], x2: &[i16], n: usize, p0: i32, p1: i32| -> i16 {
        let mut sum = ((i32::from(x1[n]) + i32::from(x1[n + 2])) + (i32::from(x1[n + 1]) << 1)) << 9;
        sum = smlawb(i32::from(x2[n + 1]) << 8, sum, p0);
        sum = smlawb(sum, i32::from(x1[n + 1]) << 11, p1);
        sat16(rshift_round(sum, 8)) as i16
    };
    for n in 0..interp {
        pred0 += delta0;
        pred1 += delta1;
        x2[n + 1] = mix(x1, x2, n, pred0, pred1);
    }
    pred0 = pred_q13[0];
    pred1 = pred_q13[1];
    for n in interp..frame_length {
        x2[n + 1] = mix(x1, x2, n, pred0, pred1);
    }
    st.pred_prev_q13 = *pred_q13;
    for n in 0..frame_length {
        let sum = i32::from(x1[n + 1]) + i32::from(x2[n + 1]);
        let diff = i32::from(x1[n + 1]) - i32::from(x2[n + 1]);
        x1[n + 1] = sat16(sum) as i16;
        x2[n + 1] = sat16(diff) as i16;
    }
}
