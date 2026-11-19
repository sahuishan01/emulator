//! One SILK channel: side information, excitation, LTP/LPC synthesis,
//! packet-loss concealment and comfort noise (RFC 6716 section 4.2.7).

use super::lpc::*;
use super::macros::*;
use super::resampler::Resampler;
use super::tables::*;
use crate::range::RangeDecoder;

pub const MAX_NB_SUBFR: usize = 4;
pub const MAX_FRAME_LENGTH: usize = 320;
pub const MAX_SUB_FRAME_LENGTH: usize = 80;
pub const LTP_ORDER: usize = 5;
pub const MAX_FRAMES_PER_PACKET: usize = 3;

pub const TYPE_NO_VOICE_ACTIVITY: i32 = 0;
pub const TYPE_VOICED: i32 = 2;

pub const CODE_INDEPENDENTLY: u8 = 0;
pub const CODE_INDEPENDENTLY_NO_LTP_SCALING: u8 = 1;
pub const CODE_CONDITIONALLY: u8 = 2;

const RAND_BUF_SIZE: usize = 128;
const RAND_BUF_MASK: i32 = 127;
const BWE_AFTER_LOSS_Q16: i32 = 63570;

#[derive(Clone, Copy, Default)]
pub struct Indices {
    pub gains: [i8; MAX_NB_SUBFR],
    pub ltp_index: [i8; MAX_NB_SUBFR],
    pub nlsf: [i8; MAX_LPC_ORDER + 1],
    pub lag_index: i16,
    pub contour_index: i8,
    pub signal_type: i8,
    pub quant_offset_type: i8,
    pub nlsf_interp_coef_q2: i8,
    pub per_index: i8,
    pub ltp_scale_index: i8,
    pub seed: i8,
}

#[derive(Clone, Copy, Default)]
pub struct Control {
    pub pitch_l: [i32; MAX_NB_SUBFR],
    pub gains_q16: [i32; MAX_NB_SUBFR],
    pub pred_coef_q12: [[i16; MAX_LPC_ORDER]; 2],
    pub ltp_coef_q14: [i16; LTP_ORDER * MAX_NB_SUBFR],
    pub ltp_scale_q14: i32,
}

#[derive(Clone)]
struct Plc {
    pitch_l_q8: i32,
    ltp_coef_q14: [i16; LTP_ORDER],
    prev_lpc_q12: [i16; MAX_LPC_ORDER],
    last_frame_lost: bool,
    rand_seed: i32,
    rand_scale_q14: i16,
    conc_energy: i32,
    conc_energy_shift: i32,
    prev_ltp_scale_q14: i16,
    prev_gain_q16: [i32; 2],
    fs_khz: usize,
    nb_subfr: usize,
    subfr_length: usize,
}

#[derive(Clone)]
struct Cng {
    exc_buf_q14: [i32; MAX_FRAME_LENGTH],
    smth_nlsf_q15: [i16; MAX_LPC_ORDER],
    synth_state: [i32; MAX_LPC_ORDER],
    smth_gain_q16: i32,
    rand_seed: i32,
    fs_khz: usize,
}

/// Per-channel SILK decoder state (`silk_decoder_state`).
#[derive(Clone)]
pub struct ChannelState {
    pub prev_gain_q16: i32,
    pub exc_q14: [i32; MAX_FRAME_LENGTH],
    pub slpc_q14_buf: [i32; MAX_LPC_ORDER],
    pub out_buf: [i16; MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH],
    pub lag_prev: i32,
    pub last_gain_index: i8,
    pub fs_khz: usize,
    pub fs_api_hz: u32,
    pub nb_subfr: usize,
    pub frame_length: usize,
    pub subfr_length: usize,
    pub ltp_mem_length: usize,
    pub lpc_order: usize,
    pub prev_nlsf_q15: [i16; MAX_LPC_ORDER],
    pub first_frame_after_reset: bool,
    pitch_lag_low_bits_icdf: &'static [u8],
    pitch_contour_icdf: &'static [u8],
    pub n_frames_decoded: usize,
    pub n_frames_per_packet: usize,
    ec_prev_signal_type: i32,
    ec_prev_lag_index: i16,
    pub vad_flags: [bool; MAX_FRAMES_PER_PACKET],
    pub lbrr_flag: bool,
    pub lbrr_flags: [bool; MAX_FRAMES_PER_PACKET],
    pub resampler: Resampler,
    nlsf_cb: &'static NlsfCodebook,
    pub indices: Indices,
    cng: Cng,
    pub loss_cnt: i32,
    pub prev_signal_type: i32,
    plc: Plc,
}

impl ChannelState {
    pub fn new() -> Self {
        let mut s = Self::blank();
        s.reset();
        s
    }

    fn blank() -> Self {
        Self {
            prev_gain_q16: 0,
            exc_q14: [0; MAX_FRAME_LENGTH],
            slpc_q14_buf: [0; MAX_LPC_ORDER],
            out_buf: [0; MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH],
            lag_prev: 0,
            last_gain_index: 0,
            fs_khz: 0,
            fs_api_hz: 0,
            nb_subfr: 0,
            frame_length: 0,
            subfr_length: 0,
            ltp_mem_length: 0,
            lpc_order: 0,
            prev_nlsf_q15: [0; MAX_LPC_ORDER],
            first_frame_after_reset: false,
            pitch_lag_low_bits_icdf: &UNIFORM8_ICDF,
            pitch_contour_icdf: &PITCH_CONTOUR_ICDF,
            n_frames_decoded: 0,
            n_frames_per_packet: 0,
            ec_prev_signal_type: 0,
            ec_prev_lag_index: 0,
            vad_flags: [false; MAX_FRAMES_PER_PACKET],
            lbrr_flag: false,
            lbrr_flags: [false; MAX_FRAMES_PER_PACKET],
            resampler: Resampler::default(),
            nlsf_cb: &NLSF_CB_NB_MB,
            indices: Indices::default(),
            cng: Cng {
                exc_buf_q14: [0; MAX_FRAME_LENGTH],
                smth_nlsf_q15: [0; MAX_LPC_ORDER],
                synth_state: [0; MAX_LPC_ORDER],
                smth_gain_q16: 0,
                rand_seed: 0,
                fs_khz: 0,
            },
            loss_cnt: 0,
            prev_signal_type: 0,
            plc: Plc {
                pitch_l_q8: 0,
                ltp_coef_q14: [0; LTP_ORDER],
                prev_lpc_q12: [0; MAX_LPC_ORDER],
                last_frame_lost: false,
                rand_seed: 0,
                rand_scale_q14: 0,
                conc_energy: 0,
                conc_energy_shift: 0,
                prev_ltp_scale_q14: 0,
                prev_gain_q16: [0; 2],
                fs_khz: 0,
                nb_subfr: 0,
                subfr_length: 0,
            },
        }
    }

    /// `silk_reset_decoder`: the reference zeroes the whole state from
    /// `prev_gain_Q16` on, i.e. everything, then re-arms CNG and PLC.
    pub fn reset(&mut self) {
        *self = Self::blank();
        self.first_frame_after_reset = true;
        self.prev_gain_q16 = 65536;
        self.cng_reset();
        self.plc_reset();
    }

    fn cng_reset(&mut self) {
        let step = i32::from(i16::MAX) / (self.lpc_order as i32 + 1);
        let mut acc = 0;
        for i in 0..self.lpc_order {
            acc += step;
            self.cng.smth_nlsf_q15[i] = acc as i16;
        }
        self.cng.smth_gain_q16 = 0;
        self.cng.rand_seed = 3_176_576;
    }

    fn plc_reset(&mut self) {
        self.plc.pitch_l_q8 = (self.frame_length as i32) << (8 - 1);
        self.plc.prev_gain_q16 = [1 << 16, 1 << 16];
        self.plc.subfr_length = 20;
        self.plc.nb_subfr = 2;
    }

    /// `silk_decoder_set_fs`.
    pub fn set_fs(&mut self, fs_khz: usize, fs_api_hz: u32) {
        self.subfr_length = 5 * fs_khz;
        let frame_length = self.nb_subfr * self.subfr_length;
        if self.fs_khz != fs_khz || self.fs_api_hz != fs_api_hz {
            self.resampler = Resampler::new(fs_khz as u32 * 1000, fs_api_hz);
            self.fs_api_hz = fs_api_hz;
        }
        if self.fs_khz != fs_khz || frame_length != self.frame_length {
            self.pitch_contour_icdf = match (fs_khz == 8, self.nb_subfr == MAX_NB_SUBFR) {
                (true, true) => &PITCH_CONTOUR_NB_ICDF,
                (true, false) => &PITCH_CONTOUR_10MS_NB_ICDF,
                (false, true) => &PITCH_CONTOUR_ICDF,
                (false, false) => &PITCH_CONTOUR_10MS_ICDF,
            };
            if self.fs_khz != fs_khz {
                self.ltp_mem_length = 20 * fs_khz;
                if fs_khz == 8 || fs_khz == 12 {
                    self.lpc_order = 10;
                    self.nlsf_cb = &NLSF_CB_NB_MB;
                } else {
                    self.lpc_order = 16;
                    self.nlsf_cb = &NLSF_CB_WB;
                }
                self.pitch_lag_low_bits_icdf = match fs_khz {
                    16 => &UNIFORM8_ICDF,
                    12 => &UNIFORM6_ICDF,
                    _ => &UNIFORM4_ICDF,
                };
                self.first_frame_after_reset = true;
                self.lag_prev = 100;
                self.last_gain_index = 10;
                self.prev_signal_type = TYPE_NO_VOICE_ACTIVITY;
                self.out_buf.fill(0);
                self.slpc_q14_buf.fill(0);
            }
            self.fs_khz = fs_khz;
            self.frame_length = frame_length;
        }
    }

    /// `silk_decode_indices`.
    pub fn decode_indices(
        &mut self,
        dec: &mut RangeDecoder,
        frame_index: usize,
        decode_lbrr: bool,
        cond_coding: u8,
    ) {
        let ix = if decode_lbrr || self.vad_flags[frame_index] {
            dec.icdf(&TYPE_OFFSET_VAD_ICDF, 8) + 2
        } else {
            dec.icdf(&TYPE_OFFSET_NO_VAD_ICDF, 8)
        };
        let ind = &mut self.indices;
        ind.signal_type = (ix >> 1) as i8;
        ind.quant_offset_type = (ix & 1) as i8;
        if cond_coding == CODE_CONDITIONALLY {
            ind.gains[0] = dec.icdf(&DELTA_GAIN_ICDF, 8) as i8;
        } else {
            let st = ind.signal_type as usize;
            ind.gains[0] = (dec.icdf(&GAIN_ICDF[st * 8..st * 8 + 8], 8) << 3) as i8;
            ind.gains[0] += dec.icdf(&UNIFORM8_ICDF, 8) as i8;
        }
        for i in 1..self.nb_subfr {
            ind.gains[i] = dec.icdf(&DELTA_GAIN_ICDF, 8) as i8;
        }
        let cb = self.nlsf_cb;
        let st1 = (ind.signal_type >> 1) as usize * cb.n_vectors;
        ind.nlsf[0] = dec.icdf(&cb.cb1_icdf[st1..], 8) as i8;
        let mut ec_ix = [0i16; MAX_LPC_ORDER];
        let mut pred_q8 = [0u8; MAX_LPC_ORDER];
        nlsf_unpack(&mut ec_ix, &mut pred_q8, cb, ind.nlsf[0] as usize);
        for i in 0..cb.order {
            let mut ix = dec.icdf(&cb.ec_icdf[ec_ix[i] as usize..], 8) as i32;
            if ix == 0 {
                ix -= dec.icdf(&NLSF_EXT_ICDF, 8) as i32;
            } else if ix == 8 {
                ix += dec.icdf(&NLSF_EXT_ICDF, 8) as i32;
            }
            ind.nlsf[i + 1] = (ix - 4) as i8;
        }
        ind.nlsf_interp_coef_q2 = if self.nb_subfr == MAX_NB_SUBFR {
            dec.icdf(&NLSF_INTERPOLATION_FACTOR_ICDF, 8) as i8
        } else {
            4
        };
        if i32::from(ind.signal_type) == TYPE_VOICED {
            let mut absolute = true;
            if cond_coding == CODE_CONDITIONALLY && self.ec_prev_signal_type == TYPE_VOICED {
                let delta = dec.icdf(&PITCH_DELTA_ICDF, 8) as i16;
                if delta > 0 {
                    ind.lag_index = self.ec_prev_lag_index + (delta - 9);
                    absolute = false;
                }
            }
            if absolute {
                ind.lag_index = (dec.icdf(&PITCH_LAG_ICDF, 8) * (self.fs_khz >> 1)) as i16;
                ind.lag_index += dec.icdf(self.pitch_lag_low_bits_icdf, 8) as i16;
            }
            self.ec_prev_lag_index = ind.lag_index;
            ind.contour_index = dec.icdf(self.pitch_contour_icdf, 8) as i8;
            ind.per_index = dec.icdf(&LTP_PER_INDEX_ICDF, 8) as i8;
            for k in 0..self.nb_subfr {
                ind.ltp_index[k] = dec.icdf(LTP_GAIN_ICDF[ind.per_index as usize], 8) as i8;
            }
            ind.ltp_scale_index = if cond_coding == CODE_INDEPENDENTLY {
                dec.icdf(&LTP_SCALE_ICDF, 8) as i8
            } else {
                0
            };
        }
        self.ec_prev_signal_type = i32::from(ind.signal_type);
        ind.seed = dec.icdf(&UNIFORM4_ICDF, 8) as i8;
    }

    /// `silk_decode_parameters`.
    fn decode_parameters(&mut self, ctrl: &mut Control, cond_coding: u8) {
        gains_dequant(
            &mut ctrl.gains_q16,
            &self.indices.gains,
            &mut self.last_gain_index,
            cond_coding == CODE_CONDITIONALLY,
            self.nb_subfr,
        );
        let order = self.lpc_order;
        let mut nlsf = [0i16; MAX_LPC_ORDER];
        nlsf_decode(&mut nlsf, &self.indices.nlsf, self.nlsf_cb);
        nlsf2a(&mut ctrl.pred_coef_q12[1][..order], &nlsf[..order]);
        if self.first_frame_after_reset {
            self.indices.nlsf_interp_coef_q2 = 4;
        }
        if self.indices.nlsf_interp_coef_q2 < 4 {
            let mut nlsf0 = [0i16; MAX_LPC_ORDER];
            let f = i32::from(self.indices.nlsf_interp_coef_q2);
            for i in 0..order {
                let p = i32::from(self.prev_nlsf_q15[i]);
                nlsf0[i] = (p + ((f * (i32::from(nlsf[i]) - p)) >> 2)) as i16;
            }
            nlsf2a(&mut ctrl.pred_coef_q12[0][..order], &nlsf0[..order]);
        } else {
            ctrl.pred_coef_q12[0] = ctrl.pred_coef_q12[1];
        }
        self.prev_nlsf_q15[..order].copy_from_slice(&nlsf[..order]);
        if self.loss_cnt != 0 {
            bwexpander(&mut ctrl.pred_coef_q12[0][..order], BWE_AFTER_LOSS_Q16);
            bwexpander(&mut ctrl.pred_coef_q12[1][..order], BWE_AFTER_LOSS_Q16);
        }
        if i32::from(self.indices.signal_type) == TYPE_VOICED {
            decode_pitch(
                self.indices.lag_index,
                self.indices.contour_index,
                &mut ctrl.pitch_l,
                self.fs_khz,
                self.nb_subfr,
            );
            let cbk = LTP_VQ[self.indices.per_index as usize];
            for k in 0..self.nb_subfr {
                let ix = self.indices.ltp_index[k] as usize;
                for i in 0..LTP_ORDER {
                    ctrl.ltp_coef_q14[k * LTP_ORDER + i] =
                        (i16::from(cbk[ix * LTP_ORDER + i])) << 7;
                }
            }
            ctrl.ltp_scale_q14 = i32::from(LTP_SCALES_Q14[self.indices.ltp_scale_index as usize]);
        } else {
            ctrl.pitch_l[..self.nb_subfr].fill(0);
            ctrl.ltp_coef_q14[..LTP_ORDER * self.nb_subfr].fill(0);
            self.indices.per_index = 0;
            ctrl.ltp_scale_q14 = 0;
        }
    }

    /// `silk_decode_core`: excitation, LTP and LPC synthesis into `xq`.
    fn decode_core(&mut self, ctrl: &mut Control, xq: &mut [i16], pulses: &[i16]) {
        let ltp_mem = self.ltp_mem_length;
        let frame_length = self.frame_length;
        let subfr_length = self.subfr_length;
        let order = self.lpc_order;
        let mut s_ltp = [0i16; MAX_FRAME_LENGTH];
        let mut s_ltp_q15 = [0i32; 2 * MAX_FRAME_LENGTH];
        let mut res_q14 = [0i32; MAX_SUB_FRAME_LENGTH];
        let mut s_lpc_q14 = [0i32; MAX_SUB_FRAME_LENGTH + MAX_LPC_ORDER];
        let offset_q10 = i32::from(
            QUANTIZATION_OFFSETS_Q10[(self.indices.signal_type >> 1) as usize * 2
                + self.indices.quant_offset_type as usize],
        );
        let nlsf_interpolation_flag = self.indices.nlsf_interp_coef_q2 < 4;
        let mut rand_seed = i32::from(self.indices.seed);
        for i in 0..frame_length {
            rand_seed = rand(rand_seed);
            let mut e = i32::from(pulses[i]) << 14;
            if e > 0 {
                e -= 80 << 4;
            } else if e < 0 {
                e += 80 << 4;
            }
            e += offset_q10 << 4;
            if rand_seed < 0 {
                e = -e;
            }
            self.exc_q14[i] = e;
            rand_seed = rand_seed.wrapping_add(i32::from(pulses[i]));
        }
        s_lpc_q14[..MAX_LPC_ORDER].copy_from_slice(&self.slpc_q14_buf);
        let mut s_ltp_buf_idx = ltp_mem;
        for k in 0..self.nb_subfr {
            let a_q12: [i16; MAX_LPC_ORDER] = ctrl.pred_coef_q12[k >> 1];
            let gain_q10 = ctrl.gains_q16[k] >> 6;
            let mut inv_gain_q31 = inverse32_varq(ctrl.gains_q16[k], 47);
            let gain_adj_q16 = if ctrl.gains_q16[k] != self.prev_gain_q16 {
                let adj = div32_varq(self.prev_gain_q16, ctrl.gains_q16[k], 16);
                for v in &mut s_lpc_q14[..MAX_LPC_ORDER] {
                    *v = smulww(adj, *v);
                }
                adj
            } else {
                1 << 16
            };
            self.prev_gain_q16 = ctrl.gains_q16[k];
            let mut signal_type = i32::from(self.indices.signal_type);
            if self.loss_cnt != 0
                && self.prev_signal_type == TYPE_VOICED
                && signal_type != TYPE_VOICED
                && k < MAX_NB_SUBFR / 2
            {
                let b = &mut ctrl.ltp_coef_q14[k * LTP_ORDER..(k + 1) * LTP_ORDER];
                b.fill(0);
                b[LTP_ORDER / 2] = 4096; // 0.25 in Q14
                signal_type = TYPE_VOICED;
                ctrl.pitch_l[k] = self.lag_prev;
            }
            let b_q14: [i16; LTP_ORDER] = ctrl.ltp_coef_q14[k * LTP_ORDER..(k + 1) * LTP_ORDER]
                .try_into()
                .expect("5 taps");
            let exc = &self.exc_q14[k * subfr_length..(k + 1) * subfr_length];
            if signal_type == TYPE_VOICED {
                let lag = ctrl.pitch_l[k] as usize;
                if k == 0 || (k == 2 && nlsf_interpolation_flag) {
                    let start_idx = ltp_mem - lag - order - LTP_ORDER / 2;
                    if k == 2 {
                        self.out_buf[ltp_mem..ltp_mem + 2 * subfr_length]
                            .copy_from_slice(&xq[..2 * subfr_length]);
                    }
                    let src_off = start_idx + k * subfr_length;
                    lpc_analysis_filter(
                        &mut s_ltp[start_idx..ltp_mem],
                        &self.out_buf[src_off..src_off + ltp_mem - start_idx],
                        &a_q12[..order],
                        ltp_mem - start_idx,
                    );
                    if k == 0 {
                        inv_gain_q31 = smulwb(inv_gain_q31, ctrl.ltp_scale_q14) << 2;
                    }
                    for i in 0..lag + LTP_ORDER / 2 {
                        s_ltp_q15[s_ltp_buf_idx - i - 1] =
                            smulwb(inv_gain_q31, i32::from(s_ltp[ltp_mem - i - 1]));
                    }
                } else if gain_adj_q16 != 1 << 16 {
                    for i in 0..lag + LTP_ORDER / 2 {
                        let p = s_ltp_buf_idx - i - 1;
                        s_ltp_q15[p] = smulww(gain_adj_q16, s_ltp_q15[p]);
                    }
                }
                let mut pred = s_ltp_buf_idx + LTP_ORDER / 2 - lag;
                for i in 0..subfr_length {
                    let w: &[i32; 5] = s_ltp_q15[pred - 4..=pred].try_into().expect("5 taps");
                    let mut ltp_pred_q13 = 2;
                    ltp_pred_q13 = smlawb(ltp_pred_q13, w[4], i32::from(b_q14[0]));
                    ltp_pred_q13 = smlawb(ltp_pred_q13, w[3], i32::from(b_q14[1]));
                    ltp_pred_q13 = smlawb(ltp_pred_q13, w[2], i32::from(b_q14[2]));
                    ltp_pred_q13 = smlawb(ltp_pred_q13, w[1], i32::from(b_q14[3]));
                    ltp_pred_q13 = smlawb(ltp_pred_q13, w[0], i32::from(b_q14[4]));
                    pred += 1;
                    res_q14[i] = exc[i].wrapping_add(ltp_pred_q13 << 1);
                    s_ltp_q15[s_ltp_buf_idx] = res_q14[i] << 1;
                    s_ltp_buf_idx += 1;
                }
            } else {
                res_q14[..subfr_length].copy_from_slice(exc);
            }
            let out = &mut xq[k * subfr_length..(k + 1) * subfr_length];
            if order == 16 {
                lpc_synth_subframe::<16>(
                    &mut s_lpc_q14,
                    &res_q14[..subfr_length],
                    &a_q12,
                    gain_q10,
                    out,
                );
            } else {
                lpc_synth_subframe::<10>(
                    &mut s_lpc_q14,
                    &res_q14[..subfr_length],
                    &a_q12,
                    gain_q10,
                    out,
                );
            }
            s_lpc_q14.copy_within(subfr_length..subfr_length + MAX_LPC_ORDER, 0);
        }
        self.slpc_q14_buf
            .copy_from_slice(&s_lpc_q14[..MAX_LPC_ORDER]);
    }

    /// `silk_decode_frame`: decode (or conceal) one frame into `out`.
    /// `lost_flag`: 0 normal, 1 lost, 2 decode LBRR.
    pub fn decode_frame(
        &mut self,
        dec: Option<&mut RangeDecoder>,
        out: &mut [i16],
        lost_flag: u8,
        cond_coding: u8,
    ) -> usize {
        let l = self.frame_length;
        let mut ctrl = Control::default();
        let decode = lost_flag == 0 || (lost_flag == 2 && self.lbrr_flags[self.n_frames_decoded]);
        match dec {
            Some(dec) if decode => {
                let mut pulses = [0i16; MAX_FRAME_LENGTH];
                self.decode_indices(dec, self.n_frames_decoded, lost_flag != 0, cond_coding);
                decode_pulses(
                    dec,
                    &mut pulses,
                    i32::from(self.indices.signal_type),
                    i32::from(self.indices.quant_offset_type),
                    l,
                );
                self.decode_parameters(&mut ctrl, cond_coding);
                self.decode_core(&mut ctrl, &mut out[..l], &pulses);
                self.update_out_buf(&out[..l]);
                self.plc(&mut ctrl, &mut out[..l], false);
                self.loss_cnt = 0;
                self.prev_signal_type = i32::from(self.indices.signal_type);
                self.first_frame_after_reset = false;
            }
            _ => {
                self.plc(&mut ctrl, &mut out[..l], true);
                self.update_out_buf(&out[..l]);
            }
        }
        self.cng(&ctrl, &mut out[..l]);
        self.plc_glue_frames(&mut out[..l]);
        self.lag_prev = ctrl.pitch_l[self.nb_subfr - 1];
        l
    }

    fn update_out_buf(&mut self, frame: &[i16]) {
        let mv = self.ltp_mem_length - self.frame_length;
        self.out_buf
            .copy_within(self.frame_length..self.frame_length + mv, 0);
        self.out_buf[mv..mv + self.frame_length].copy_from_slice(frame);
    }

    fn plc(&mut self, ctrl: &mut Control, frame: &mut [i16], lost: bool) {
        if self.fs_khz != self.plc.fs_khz {
            self.plc_reset();
            self.plc.fs_khz = self.fs_khz;
        }
        if lost {
            self.plc_conceal(ctrl, frame);
            self.loss_cnt += 1;
        } else {
            self.plc_update(ctrl);
        }
    }

    fn plc_update(&mut self, ctrl: &Control) {
        let nb = self.nb_subfr;
        self.prev_signal_type = i32::from(self.indices.signal_type);
        let mut ltp_gain_q14 = 0i32;
        if i32::from(self.indices.signal_type) == TYPE_VOICED {
            let mut j = 0;
            while ((j * self.subfr_length) as i32) < ctrl.pitch_l[nb - 1] {
                if j == nb {
                    break;
                }
                let base = (nb - 1 - j) * LTP_ORDER;
                let temp: i32 = ctrl.ltp_coef_q14[base..base + LTP_ORDER]
                    .iter()
                    .map(|&v| i32::from(v))
                    .sum();
                if temp > ltp_gain_q14 {
                    ltp_gain_q14 = temp;
                    self.plc
                        .ltp_coef_q14
                        .copy_from_slice(&ctrl.ltp_coef_q14[base..base + LTP_ORDER]);
                    self.plc.pitch_l_q8 = ctrl.pitch_l[nb - 1 - j] << 8;
                }
                j += 1;
            }
            self.plc.ltp_coef_q14 = [0; LTP_ORDER];
            self.plc.ltp_coef_q14[LTP_ORDER / 2] = ltp_gain_q14 as i16;
            if ltp_gain_q14 < 11469 {
                let scale_q10 = (11469 << 10) / ltp_gain_q14.max(1);
                for v in &mut self.plc.ltp_coef_q14 {
                    *v = (smulbb(i32::from(*v), scale_q10) >> 10) as i16;
                }
            } else if ltp_gain_q14 > 15565 {
                let scale_q14 = (15565 << 14) / ltp_gain_q14.max(1);
                for v in &mut self.plc.ltp_coef_q14 {
                    *v = (smulbb(i32::from(*v), scale_q14) >> 14) as i16;
                }
            }
        } else {
            self.plc.pitch_l_q8 = smulbb(self.fs_khz as i32, 18) << 8;
            self.plc.ltp_coef_q14 = [0; LTP_ORDER];
        }
        self.plc.prev_lpc_q12[..self.lpc_order]
            .copy_from_slice(&ctrl.pred_coef_q12[1][..self.lpc_order]);
        self.plc.prev_ltp_scale_q14 = ctrl.ltp_scale_q14 as i16;
        self.plc
            .prev_gain_q16
            .copy_from_slice(&ctrl.gains_q16[nb - 2..nb]);
        self.plc.subfr_length = self.subfr_length;
        self.plc.nb_subfr = nb;
    }

    fn plc_conceal(&mut self, ctrl: &mut Control, frame: &mut [i16]) {
        const HARM_ATT_Q15: [i32; 2] = [32440, 31130];
        const RAND_ATT_V_Q15: [i32; 2] = [31130, 26214];
        const RAND_ATT_UV_Q15: [i32; 2] = [32440, 29491];
        let ltp_mem = self.ltp_mem_length;
        let order = self.lpc_order;
        let mut s_ltp = [0i16; MAX_FRAME_LENGTH];
        let mut s_ltp_q14 = [0i32; 2 * MAX_FRAME_LENGTH];
        let prev_gain_q10 = [
            self.plc.prev_gain_q16[0] >> 6,
            self.plc.prev_gain_q16[1] >> 6,
        ];
        if self.first_frame_after_reset {
            self.plc.prev_lpc_q12 = [0; MAX_LPC_ORDER];
        }
        // Energies of the last two subframes of excitation.
        let sl = self.subfr_length;
        let nb = self.nb_subfr;
        let mut exc_buf = [0i16; 2 * MAX_SUB_FRAME_LENGTH];
        for k in 0..2 {
            for i in 0..sl {
                exc_buf[k * sl + i] =
                    sat16(smulww(self.exc_q14[i + (k + nb - 2) * sl], prev_gain_q10[k]) >> 8)
                        as i16;
            }
        }
        let (energy1, shift1) = sum_sqr_shift(&exc_buf[..sl]);
        let (energy2, shift2) = sum_sqr_shift(&exc_buf[sl..2 * sl]);
        let rand_off = if (energy1 >> shift2) < (energy2 >> shift1) {
            ((self.plc.nb_subfr - 1) * self.plc.subfr_length).saturating_sub(RAND_BUF_SIZE)
        } else {
            (self.plc.nb_subfr * self.plc.subfr_length).saturating_sub(RAND_BUF_SIZE)
        };
        let mut b_q14 = self.plc.ltp_coef_q14;
        let mut rand_scale_q14 = i32::from(self.plc.rand_scale_q14);
        let att = (self.loss_cnt as usize).min(1);
        let harm_gain_q15 = HARM_ATT_Q15[att];
        let mut rand_gain_q15 = if self.prev_signal_type == TYPE_VOICED {
            RAND_ATT_V_Q15[att]
        } else {
            RAND_ATT_UV_Q15[att]
        };
        bwexpander(&mut self.plc.prev_lpc_q12[..order], 64881); // SILK_FIX_CONST(0.99, 16)
        let a_q12 = self.plc.prev_lpc_q12;
        if self.loss_cnt == 0 {
            rand_scale_q14 = 1 << 14;
            if self.prev_signal_type == TYPE_VOICED {
                for &b in &b_q14 {
                    rand_scale_q14 -= i32::from(b);
                }
                rand_scale_q14 = rand_scale_q14.max(3277) as i16 as i32;
                rand_scale_q14 = (smulbb(rand_scale_q14, i32::from(self.plc.prev_ltp_scale_q14))
                    >> 14) as i16 as i32;
            } else {
                let inv_gain_q30 = lpc_inverse_pred_gain(&self.plc.prev_lpc_q12[..order]);
                let mut down = ((1i32 << 30) >> 3).min(inv_gain_q30);
                down = ((1i32 << 30) >> 8).max(down);
                down <<= 3;
                rand_gain_q15 = smulwb(down, rand_gain_q15) >> 14;
            }
        }
        let mut rand_seed = self.plc.rand_seed;
        let mut lag = rshift_round(self.plc.pitch_l_q8, 8) as usize;
        let mut s_ltp_buf_idx = ltp_mem;
        let idx = ltp_mem - lag - order - LTP_ORDER / 2;
        lpc_analysis_filter(
            &mut s_ltp[idx..ltp_mem],
            &self.out_buf[idx..ltp_mem],
            &a_q12[..order],
            ltp_mem - idx,
        );
        let inv_gain_q30 = inverse32_varq(self.plc.prev_gain_q16[1], 46).min(i32::MAX >> 1);
        for i in idx + order..ltp_mem {
            s_ltp_q14[i] = smulwb(inv_gain_q30, i32::from(s_ltp[i]));
        }
        let rand_ptr = &self.exc_q14[rand_off..];
        for _ in 0..nb {
            let mut pred = s_ltp_buf_idx + LTP_ORDER / 2 - lag;
            for _ in 0..sl {
                let mut ltp_pred_q12 = 2;
                ltp_pred_q12 = smlawb(ltp_pred_q12, s_ltp_q14[pred], i32::from(b_q14[0]));
                ltp_pred_q12 = smlawb(ltp_pred_q12, s_ltp_q14[pred - 1], i32::from(b_q14[1]));
                ltp_pred_q12 = smlawb(ltp_pred_q12, s_ltp_q14[pred - 2], i32::from(b_q14[2]));
                ltp_pred_q12 = smlawb(ltp_pred_q12, s_ltp_q14[pred - 3], i32::from(b_q14[3]));
                ltp_pred_q12 = smlawb(ltp_pred_q12, s_ltp_q14[pred - 4], i32::from(b_q14[4]));
                pred += 1;
                rand_seed = rand(rand_seed);
                let ri = ((rand_seed >> 25) & RAND_BUF_MASK) as usize;
                s_ltp_q14[s_ltp_buf_idx] = smlawb(ltp_pred_q12, rand_ptr[ri], rand_scale_q14) << 2;
                s_ltp_buf_idx += 1;
            }
            for b in &mut b_q14 {
                *b = (smulbb(harm_gain_q15, i32::from(*b)) >> 15) as i16;
            }
            rand_scale_q14 = smulbb(rand_scale_q14, rand_gain_q15) >> 15;
            self.plc.pitch_l_q8 = smlawb(self.plc.pitch_l_q8, self.plc.pitch_l_q8, 655);
            self.plc.pitch_l_q8 = self.plc.pitch_l_q8.min(smulbb(18, self.fs_khz as i32) << 8);
            lag = rshift_round(self.plc.pitch_l_q8, 8) as usize;
        }
        // LPC synthesis over the LTP output (state lives just before it).
        let base = ltp_mem - MAX_LPC_ORDER;
        s_ltp_q14[base..ltp_mem].copy_from_slice(&self.slpc_q14_buf);
        for i in 0..self.frame_length {
            let at = base + MAX_LPC_ORDER + i;
            let p = if order == 16 {
                lpc_pred::<16>(&s_ltp_q14, at, &a_q12)
            } else {
                lpc_pred::<10>(&s_ltp_q14, at, &a_q12)
            };
            s_ltp_q14[at] = add_sat32(s_ltp_q14[at], lshift_sat32(p, 4));
            frame[i] = sat16(sat16(rshift_round(
                smulww(s_ltp_q14[at], prev_gain_q10[1]),
                8,
            ))) as i16;
        }
        self.slpc_q14_buf.copy_from_slice(
            &s_ltp_q14[base + self.frame_length..base + self.frame_length + MAX_LPC_ORDER],
        );
        self.plc.rand_seed = rand_seed;
        self.plc.rand_scale_q14 = rand_scale_q14 as i16;
        self.plc.ltp_coef_q14 = b_q14;
        for p in &mut ctrl.pitch_l {
            *p = lag as i32;
        }
    }

    fn plc_glue_frames(&mut self, frame: &mut [i16]) {
        if self.loss_cnt != 0 {
            let (e, s) = sum_sqr_shift(frame);
            self.plc.conc_energy = e;
            self.plc.conc_energy_shift = s;
            self.plc.last_frame_lost = true;
        } else {
            if self.plc.last_frame_lost {
                let (mut energy, energy_shift) = sum_sqr_shift(frame);
                if energy_shift > self.plc.conc_energy_shift {
                    self.plc.conc_energy >>= energy_shift - self.plc.conc_energy_shift;
                } else if energy_shift < self.plc.conc_energy_shift {
                    energy >>= self.plc.conc_energy_shift - energy_shift;
                }
                if energy > self.plc.conc_energy {
                    let lz = clz32(self.plc.conc_energy) - 1;
                    self.plc.conc_energy <<= lz;
                    energy >>= 0.max(24 - lz);
                    let frac_q24 = self.plc.conc_energy / energy.max(1);
                    let mut gain_q16 = sqrt_approx(frac_q24) << 4;
                    let mut slope_q16 = ((1i32 << 16) - gain_q16) / frame.len() as i32;
                    slope_q16 <<= 2;
                    for v in frame.iter_mut() {
                        *v = smulwb(gain_q16, i32::from(*v)) as i16;
                        gain_q16 += slope_q16;
                        if gain_q16 > 1 << 16 {
                            break;
                        }
                    }
                }
            }
            self.plc.last_frame_lost = false;
        }
    }

    fn cng(&mut self, ctrl: &Control, frame: &mut [i16]) {
        let order = self.lpc_order;
        if self.fs_khz != self.cng.fs_khz {
            self.cng_reset();
            self.cng.fs_khz = self.fs_khz;
        }
        if self.loss_cnt == 0 && self.prev_signal_type == TYPE_NO_VOICE_ACTIVITY {
            for i in 0..order {
                let d = i32::from(self.prev_nlsf_q15[i]) - i32::from(self.cng.smth_nlsf_q15[i]);
                self.cng.smth_nlsf_q15[i] =
                    (i32::from(self.cng.smth_nlsf_q15[i]) + smulwb(d, 16348)) as i16;
            }
            let mut max_gain = 0;
            let mut subfr = 0;
            for i in 0..self.nb_subfr {
                if ctrl.gains_q16[i] > max_gain {
                    max_gain = ctrl.gains_q16[i];
                    subfr = i;
                }
            }
            let sl = self.subfr_length;
            self.cng
                .exc_buf_q14
                .copy_within(0..(self.nb_subfr - 1) * sl, sl);
            self.cng.exc_buf_q14[..sl].copy_from_slice(&self.exc_q14[subfr * sl..(subfr + 1) * sl]);
            for i in 0..self.nb_subfr {
                self.cng.smth_gain_q16 += smulwb(ctrl.gains_q16[i] - self.cng.smth_gain_q16, 4634);
                if smulww(self.cng.smth_gain_q16, 46396) > ctrl.gains_q16[i] {
                    self.cng.smth_gain_q16 = ctrl.gains_q16[i];
                }
            }
        }
        if self.loss_cnt != 0 {
            let length = frame.len();
            let mut sig = [0i32; MAX_FRAME_LENGTH + MAX_LPC_ORDER];
            let mut gain_q16 = smulww(
                i32::from(self.plc.rand_scale_q14),
                self.plc.prev_gain_q16[1],
            );
            if gain_q16 >= 1 << 21 || self.cng.smth_gain_q16 > 1 << 23 {
                gain_q16 = smultt(gain_q16, gain_q16);
                gain_q16 = smultt(self.cng.smth_gain_q16, self.cng.smth_gain_q16)
                    .wrapping_sub(gain_q16 << 5);
                gain_q16 = sqrt_approx(gain_q16) << 16;
            } else {
                gain_q16 = smulww(gain_q16, gain_q16);
                gain_q16 = smulww(self.cng.smth_gain_q16, self.cng.smth_gain_q16)
                    .wrapping_sub(gain_q16 << 5);
                gain_q16 = sqrt_approx(gain_q16) << 8;
            }
            let gain_q10 = gain_q16 >> 6;
            // Excitation from the stored buffer.
            let mut exc_mask = 255usize;
            while exc_mask > length {
                exc_mask >>= 1;
            }
            let mut seed = self.cng.rand_seed;
            for i in 0..length {
                seed = rand(seed);
                let idx = ((seed >> 24) as usize) & exc_mask;
                sig[MAX_LPC_ORDER + i] = self.cng.exc_buf_q14[idx];
            }
            self.cng.rand_seed = seed;
            let mut a_q12 = [0i16; MAX_LPC_ORDER];
            nlsf2a(&mut a_q12[..order], &self.cng.smth_nlsf_q15[..order]);
            sig[..MAX_LPC_ORDER].copy_from_slice(&self.cng.synth_state);
            for i in 0..length {
                let p = if order == 16 {
                    lpc_pred::<16>(&sig, MAX_LPC_ORDER + i, &a_q12)
                } else {
                    lpc_pred::<10>(&sig, MAX_LPC_ORDER + i, &a_q12)
                };
                sig[MAX_LPC_ORDER + i] = add_sat32(sig[MAX_LPC_ORDER + i], lshift_sat32(p, 4));
                let add = sat16(rshift_round(smulww(sig[MAX_LPC_ORDER + i], gain_q10), 8));
                frame[i] = sat16(i32::from(frame[i]) + add) as i16;
            }
            self.cng
                .synth_state
                .copy_from_slice(&sig[length..length + MAX_LPC_ORDER]);
        } else {
            self.cng.synth_state[..order].fill(0);
        }
    }
}

/// Short-term prediction from the `N` samples before `at` (Q10 result with
/// the reference's rounding bias). Monomorphised for orders 10 and 16 so the
/// compiler unrolls it without bounds checks, like the reference's unrolled C.
#[inline(always)]
fn lpc_pred<const N: usize>(state: &[i32], at: usize, a_q12: &[i16; MAX_LPC_ORDER]) -> i32 {
    let hist: &[i32; N] = state[at - N..at].try_into().expect("history window");
    let mut pred = (N >> 1) as i32;
    for j in 0..N {
        pred = smlawb(pred, hist[N - 1 - j], i32::from(a_q12[j]));
    }
    pred
}

#[inline(always)]
fn lpc_synth_subframe<const N: usize>(
    s_lpc: &mut [i32],
    res: &[i32],
    a_q12: &[i16; MAX_LPC_ORDER],
    gain_q10: i32,
    out: &mut [i16],
) {
    for (i, (o, &r)) in out.iter_mut().zip(res).enumerate() {
        let at = MAX_LPC_ORDER + i;
        let pred = lpc_pred::<N>(s_lpc, at, a_q12);
        let v = add_sat32(r, lshift_sat32(pred, 4));
        s_lpc[at] = v;
        *o = sat16(rshift_round(smulww(v, gain_q10), 8)) as i16;
    }
}

fn gains_dequant(
    gain_q16: &mut [i32; MAX_NB_SUBFR],
    ind: &[i8; MAX_NB_SUBFR],
    prev_ind: &mut i8,
    conditional: bool,
    nb_subfr: usize,
) {
    const OFFSET: i32 = (2 * 128) / 6 + 16 * 128;
    const INV_SCALE_Q16: i32 = (65536 * (((88 - 2) * 128) / 6)) / (64 - 1);
    for k in 0..nb_subfr {
        let mut p = i32::from(*prev_ind);
        if k == 0 && !conditional {
            p = i32::from(ind[k]).max(p - 16);
        } else {
            let ind_tmp = i32::from(ind[k]) - 4;
            let thr = 2 * 36 - 64 + p;
            if ind_tmp > thr {
                p += (ind_tmp << 1) - thr;
            } else {
                p += ind_tmp;
            }
        }
        p = p.clamp(0, 63);
        *prev_ind = p as i8;
        gain_q16[k] = log2lin((smulwb(INV_SCALE_Q16, p) + OFFSET).min(3967));
    }
}

fn decode_pitch(
    lag_index: i16,
    contour_index: i8,
    pitch_lags: &mut [i32; MAX_NB_SUBFR],
    fs_khz: usize,
    nb_subfr: usize,
) {
    let (cb, cbk_size): (&[i8], usize) = match (fs_khz == 8, nb_subfr == MAX_NB_SUBFR) {
        (true, true) => (&CB_LAGS_STAGE2, 11),
        (true, false) => (&CB_LAGS_STAGE2_10MS, 3),
        (false, true) => (&CB_LAGS_STAGE3, 34),
        (false, false) => (&CB_LAGS_STAGE3_10MS, 12),
    };
    let min_lag = 2 * fs_khz as i32;
    let max_lag = 18 * fs_khz as i32;
    let lag = min_lag + i32::from(lag_index);
    for k in 0..nb_subfr {
        let v = lag + i32::from(cb[k * cbk_size + contour_index as usize]);
        pitch_lags[k] = limit(v, min_lag, max_lag);
    }
}

fn decode_split(dec: &mut RangeDecoder, p: i16, table: &[u8]) -> (i16, i16) {
    if p > 0 {
        let off = usize::from(SHELL_CODE_TABLE_OFFSETS[p as usize]);
        let c1 = dec.icdf(&table[off..], 8) as i16;
        (c1, p - c1)
    } else {
        (0, 0)
    }
}

fn shell_decoder(out: &mut [i16], dec: &mut RangeDecoder, pulses4: i16) {
    let (a, b) = decode_split(dec, pulses4, &SHELL_CODE_TABLE3);
    let p3 = [a, b];
    let mut p2 = [0i16; 4];
    let mut p1 = [0i16; 8];
    (p2[0], p2[1]) = decode_split(dec, p3[0], &SHELL_CODE_TABLE2);
    (p1[0], p1[1]) = decode_split(dec, p2[0], &SHELL_CODE_TABLE1);
    (out[0], out[1]) = decode_split(dec, p1[0], &SHELL_CODE_TABLE0);
    (out[2], out[3]) = decode_split(dec, p1[1], &SHELL_CODE_TABLE0);
    (p1[2], p1[3]) = decode_split(dec, p2[1], &SHELL_CODE_TABLE1);
    (out[4], out[5]) = decode_split(dec, p1[2], &SHELL_CODE_TABLE0);
    (out[6], out[7]) = decode_split(dec, p1[3], &SHELL_CODE_TABLE0);
    (p2[2], p2[3]) = decode_split(dec, p3[1], &SHELL_CODE_TABLE2);
    (p1[4], p1[5]) = decode_split(dec, p2[2], &SHELL_CODE_TABLE1);
    (out[8], out[9]) = decode_split(dec, p1[4], &SHELL_CODE_TABLE0);
    (out[10], out[11]) = decode_split(dec, p1[5], &SHELL_CODE_TABLE0);
    (p1[6], p1[7]) = decode_split(dec, p2[3], &SHELL_CODE_TABLE1);
    (out[12], out[13]) = decode_split(dec, p1[6], &SHELL_CODE_TABLE0);
    (out[14], out[15]) = decode_split(dec, p1[7], &SHELL_CODE_TABLE0);
}

/// `silk_decode_pulses`: rate level, pulse counts, shell coding, LSBs and
/// signs.
pub fn decode_pulses(
    dec: &mut RangeDecoder,
    pulses: &mut [i16],
    signal_type: i32,
    quant_offset_type: i32,
    frame_length: usize,
) {
    let rate_level = dec.icdf(&RATE_LEVELS_ICDF[(signal_type >> 1) as usize * 9..], 8);
    let mut iter = frame_length >> 4;
    if iter * 16 < frame_length {
        iter += 1;
    }
    let mut sum_pulses = [0i32; 20];
    let mut n_lshifts = [0i32; 20];
    for i in 0..iter {
        sum_pulses[i] = dec.icdf(&PULSES_PER_BLOCK_ICDF[rate_level * 18..], 8) as i32;
        while sum_pulses[i] == 17 {
            n_lshifts[i] += 1;
            let off = 9 * 18 + usize::from(n_lshifts[i] == 10);
            sum_pulses[i] = dec.icdf(&PULSES_PER_BLOCK_ICDF[off..], 8) as i32;
        }
    }
    for i in 0..iter {
        let block = &mut pulses[i * 16..(i + 1) * 16];
        if sum_pulses[i] > 0 {
            shell_decoder(block, dec, sum_pulses[i] as i16);
        } else {
            block.fill(0);
        }
    }
    for i in 0..iter {
        if n_lshifts[i] > 0 {
            let nls = n_lshifts[i];
            for v in &mut pulses[i * 16..(i + 1) * 16] {
                let mut abs_q = i32::from(*v);
                for _ in 0..nls {
                    abs_q = (abs_q << 1) + dec.icdf(&LSB_ICDF, 8) as i32;
                }
                *v = abs_q as i16;
            }
            sum_pulses[i] |= nls << 5;
        }
    }
    // Signs.
    let base = 7 * (quant_offset_type + (signal_type << 1)) as usize;
    let icdf_tab = &SIGN_ICDF[base..];
    let blocks = (frame_length + 8) >> 4;
    for i in 0..blocks {
        let p = sum_pulses[i];
        if p > 0 {
            let icdf = [icdf_tab[(p & 0x1f).min(6) as usize], 0];
            for v in &mut pulses[i * 16..(i + 1) * 16] {
                if *v > 0 {
                    let s = dec.icdf(&icdf, 8) as i16;
                    *v *= (s << 1) - 1;
                }
            }
        }
    }
}
