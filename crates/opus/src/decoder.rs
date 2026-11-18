//! Opus decoder state machine (RFC 6716 section 4): mode switching between
//! SILK, hybrid and CELT, redundant CELT frames and transition cross-fades.

use crate::celt::{self, CeltDecoder};
use crate::packet::{Bandwidth, Mode, Packet, Toc};
use crate::range::RangeDecoder;
use crate::silk::{SilkControl, SilkDecoder};
use crate::Error;

/// Single-stream Opus decoder producing interleaved PCM.
#[derive(Clone)]
pub struct Decoder {
    rate: u32,
    channels: usize,
    celt: CeltDecoder,
    silk: SilkDecoder,
    stream_channels: usize,
    bandwidth: Option<Bandwidth>,
    mode: Option<Mode>,
    prev_mode: Option<Mode>,
    frame_size: usize,
    prev_redundancy: bool,
    last_packet_duration: usize,
    range_final: u32,
    softclip_mem: [f32; 2],
    pcm: Vec<f32>,
    /// SILK control kept across lost packets (`DecControl` in the reference).
    silk_rate: u32,
    silk_channels: usize,
    /// Reused SILK output staging buffer.
    silk_pcm: Vec<i16>,
}

impl Decoder {
    /// `rate` is 8, 12, 16, 24 or 48 kHz; `channels` is 1 or 2.
    pub fn new(rate: u32, channels: usize) -> Result<Self, Error> {
        if !matches!(rate, 8000 | 12000 | 16000 | 24000 | 48000) || !(1..=2).contains(&channels) {
            return Err(Error::BadArgument);
        }
        Ok(Self {
            rate,
            channels,
            celt: CeltDecoder::new(rate, channels)?,
            silk: SilkDecoder::new(),
            stream_channels: channels,
            bandwidth: None,
            mode: None,
            prev_mode: None,
            frame_size: rate as usize / 400,
            prev_redundancy: false,
            last_packet_duration: 0,
            range_final: 0,
            softclip_mem: [0.0; 2],
            pcm: Vec::new(),
            silk_rate: 16000,
            silk_channels: channels,
            silk_pcm: Vec::new(),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Range coder state after the last decoded packet (for conformance).
    pub fn final_range(&self) -> u32 {
        self.range_final
    }

    pub fn last_packet_duration(&self) -> usize {
        self.last_packet_duration
    }

    /// `OPUS_RESET_STATE`.
    pub fn reset(&mut self) {
        self.celt.reset();
        self.silk.reset();
        self.stream_channels = self.channels;
        self.bandwidth = None;
        self.mode = None;
        self.prev_mode = None;
        self.frame_size = self.rate as usize / 400;
        self.prev_redundancy = false;
        self.last_packet_duration = 0;
        self.range_final = 0;
        self.softclip_mem = [0.0; 2];
    }

    /// Decode a packet (or conceal a lost one with `None`) into interleaved
    /// 16-bit PCM. `frame_size` is the capacity of `pcm` per channel. Returns
    /// the number of samples per channel.
    pub fn decode(&mut self, packet: Option<&[u8]>, pcm: &mut [i16], frame_size: usize) -> Result<usize, Error> {
        let n = frame_size.min(pcm.len() / self.channels);
        let mut buf = std::mem::take(&mut self.pcm);
        buf.resize(n * self.channels, 0.0);
        let res = self.decode_native(packet, &mut buf, n, false, true);
        if let Ok(samples) = res {
            for (o, &v) in pcm.iter_mut().zip(&buf[..samples * self.channels]) {
                *o = float_to_i16(v);
            }
        }
        self.pcm = buf;
        res
    }

    /// Recover the previous (lost) frame from in-band FEC data in `packet`,
    /// concealing whatever FEC cannot cover. `frame_size` must be a multiple
    /// of 2.5 ms.
    pub fn decode_fec(&mut self, packet: &[u8], pcm: &mut [i16], frame_size: usize) -> Result<usize, Error> {
        let n = frame_size.min(pcm.len() / self.channels);
        if n % (self.rate as usize / 400) != 0 {
            return Err(Error::BadArgument);
        }
        let mut buf = std::mem::take(&mut self.pcm);
        buf.resize(n * self.channels, 0.0);
        let res = self.decode_native(Some(packet), &mut buf, n, true, true);
        if let Ok(samples) = res {
            for (o, &v) in pcm.iter_mut().zip(&buf[..samples * self.channels]) {
                *o = float_to_i16(v);
            }
        }
        self.pcm = buf;
        res
    }

    /// Float output variant (full scale 1.0, no soft clipping).
    pub fn decode_float(&mut self, packet: Option<&[u8]>, pcm: &mut [f32], frame_size: usize) -> Result<usize, Error> {
        let n = frame_size.min(pcm.len() / self.channels);
        self.decode_native(packet, &mut pcm[..n * self.channels], n, false, false)
    }

    /// Decode one (possibly self-delimited) stream of a multistream packet
    /// into float PCM. An empty `data` conceals. Returns (samples, bytes
    /// consumed).
    pub(crate) fn decode_stream(
        &mut self,
        data: &[u8],
        self_delimited: bool,
        pcm: &mut [f32],
        frame_size: usize,
    ) -> Result<(usize, usize), Error> {
        if data.is_empty() {
            return Ok((self.decode_native(None, pcm, frame_size, false, true)?, 0));
        }
        let packet = Packet::parse(data, self_delimited)?;
        let samples = self.decode_parsed(data, &packet, pcm, frame_size, false)?;
        soft_clip_pcm(&mut pcm[..samples * self.channels], samples, self.channels, &mut self.softclip_mem);
        Ok((samples, packet.consumed))
    }

    fn decode_native(
        &mut self,
        data: Option<&[u8]>,
        pcm: &mut [f32],
        frame_size: usize,
        fec: bool,
        soft_clip: bool,
    ) -> Result<usize, Error> {
        let data = data.filter(|d| !d.is_empty());
        let Some(data) = data else {
            if frame_size % (self.rate as usize / 400) != 0 {
                return Err(Error::BadArgument);
            }
            let mut count = 0;
            while count < frame_size {
                let ret = self.decode_frame(None, &mut pcm[count * self.channels..], frame_size - count, false)?;
                count += ret;
            }
            self.last_packet_duration = count;
            return Ok(count);
        };
        let packet = Packet::parse(data, false)?;
        let samples = self.decode_parsed(data, &packet, pcm, frame_size, fec)?;
        if fec {
            // The reference returns from the FEC path before soft clipping.
            return Ok(samples);
        }
        if soft_clip {
            soft_clip_pcm(&mut pcm[..samples * self.channels], samples, self.channels, &mut self.softclip_mem);
        } else {
            self.softclip_mem = [0.0; 2];
        }
        Ok(samples)
    }

    fn decode_parsed(
        &mut self,
        data: &[u8],
        packet: &Packet,
        pcm: &mut [f32],
        frame_size: usize,
        fec: bool,
    ) -> Result<usize, Error> {
        let toc: Toc = packet.toc;
        let packet_frame_size = toc.frame_size(self.rate);
        if fec {
            if frame_size < packet_frame_size || toc.mode == Mode::CeltOnly || self.mode == Some(Mode::CeltOnly) {
                return self.decode_native(None, pcm, frame_size, false, false);
            }
            let duration_copy = self.last_packet_duration;
            if frame_size > packet_frame_size {
                if let Err(e) = self.decode_native(None, pcm, frame_size - packet_frame_size, false, false) {
                    self.last_packet_duration = duration_copy;
                    return Err(e);
                }
            }
            self.mode = Some(toc.mode);
            self.bandwidth = Some(toc.bandwidth);
            self.frame_size = packet_frame_size;
            self.stream_channels = if toc.stereo { 2 } else { 1 };
            let first = &data[packet.frames.get(0)];
            let at = self.channels * (frame_size - packet_frame_size);
            self.decode_frame(Some(first), &mut pcm[at..], packet_frame_size, true)?;
            self.last_packet_duration = frame_size;
            return Ok(frame_size);
        }
        if packet.frames.len() * packet_frame_size > frame_size {
            return Err(Error::BufferTooSmall);
        }
        self.mode = Some(toc.mode);
        self.bandwidth = Some(toc.bandwidth);
        self.frame_size = packet_frame_size;
        self.stream_channels = if toc.stereo { 2 } else { 1 };
        let mut nb = 0;
        for range in packet.frames.iter() {
            let frame = &data[range];
            let ret = self.decode_frame(Some(frame), &mut pcm[nb * self.channels..], frame_size - nb, false)?;
            nb += ret;
        }
        self.last_packet_duration = nb;
        Ok(nb)
    }

    /// One Opus frame (`opus_decode_frame`). `data` of length <= 1 means a
    /// lost frame.
    fn decode_frame(&mut self, data: Option<&[u8]>, pcm: &mut [f32], frame_size: usize, decode_fec: bool) -> Result<usize, Error> {
        let fs = self.rate as usize;
        let ch = self.channels;
        let f20 = fs / 50;
        let f10 = f20 >> 1;
        let f5 = f10 >> 1;
        let f2_5 = f5 >> 1;
        if frame_size < f2_5 {
            return Err(Error::BufferTooSmall);
        }
        let mut frame_size = frame_size.min(fs / 25 * 3);
        let data = match data {
            Some(d) if d.len() > 1 => Some(d),
            _ => {
                frame_size = frame_size.min(self.frame_size);
                None
            }
        };
        let len = data.map_or(0, <[u8]>::len);
        let mut audiosize;
        let mode;
        let bandwidth;
        let mut range_dec = data.map(RangeDecoder::new);
        if data.is_some() {
            audiosize = self.frame_size;
            mode = self.mode.expect("mode set by packet");
            bandwidth = self.bandwidth;
        } else {
            audiosize = frame_size;
            let plc_mode = if self.prev_redundancy { Some(Mode::CeltOnly) } else { self.prev_mode };
            bandwidth = None;
            let Some(m) = plc_mode else {
                pcm[..audiosize * ch].fill(0.0);
                return Ok(audiosize);
            };
            mode = m;
            if audiosize > f20 {
                let mut done = 0;
                while audiosize > 0 {
                    let ret = self.decode_frame(None, &mut pcm[done * ch..], audiosize.min(f20), false)?;
                    done += ret;
                    audiosize -= ret;
                }
                return Ok(frame_size);
            } else if audiosize < f20 {
                if audiosize > f10 {
                    audiosize = f10;
                } else if mode != Mode::SilkOnly && audiosize > f5 && audiosize < f10 {
                    audiosize = f5;
                }
            }
        }

        let mut transition = false;
        // At most 5 ms of 48 kHz stereo for transitions and redundancy.
        let mut pcm_transition = [0f32; 480];
        if data.is_some()
            && self.prev_mode.is_some()
            && ((mode == Mode::CeltOnly && self.prev_mode != Some(Mode::CeltOnly) && !self.prev_redundancy)
                || (mode != Mode::CeltOnly && self.prev_mode == Some(Mode::CeltOnly)))
        {
            transition = true;
            if mode == Mode::CeltOnly {
                self.decode_frame(None, &mut pcm_transition[..f5 * ch], f5.min(audiosize), false)?;
            }
        }
        if audiosize > frame_size {
            return Err(Error::BadArgument);
        }
        frame_size = audiosize;

        let mut pcm_silk = std::mem::take(&mut self.silk_pcm);
        if mode != Mode::CeltOnly {
            pcm_silk.clear();
            pcm_silk.resize(f10.max(frame_size) * ch, 0);
            if self.prev_mode == Some(Mode::CeltOnly) {
                self.silk.reset();
            }
            let payload_ms = 10.max(1000 * audiosize / fs);
            if data.is_some() {
                self.silk_rate = match (mode, bandwidth) {
                    (Mode::SilkOnly, Some(Bandwidth::Narrow)) => 8000,
                    (Mode::SilkOnly, Some(Bandwidth::Medium)) => 12000,
                    _ => 16000,
                };
                self.silk_channels = self.stream_channels;
            }
            let internal_rate = self.silk_rate;
            let ctl = SilkControl {
                channels_api: ch,
                channels_internal: self.silk_channels,
                api_rate: self.rate,
                internal_rate,
                payload_ms,
            };
            let lost_flag = if data.is_none() { 1 } else { 2 * u8::from(decode_fec) };
            let mut decoded = 0;
            loop {
                let first = decoded == 0;
                let out = &mut pcm_silk[decoded * ch..];
                let n = match self.silk.decode(&ctl, lost_flag, first, range_dec.as_mut(), out) {
                    Ok(n) => n,
                    Err(e) => {
                        if lost_flag != 0 {
                            let k = (frame_size * ch).min(out.len());
                            out[..k].fill(0);
                            frame_size
                        } else {
                            return Err(e);
                        }
                    }
                };
                decoded += n;
                if decoded >= frame_size {
                    break;
                }
            }
        }

        let mut start_band = 0;
        let mut redundancy = false;
        let mut redundancy_bytes = 0usize;
        let mut celt_to_silk = false;
        let mut len = len;
        if !decode_fec && mode != Mode::CeltOnly {
            if let Some(dec) = range_dec.as_mut() {
                if dec.tell() + 17 + 20 * i32::from(mode == Mode::Hybrid) <= 8 * len as i32 {
                    redundancy = if mode == Mode::Hybrid { dec.bit_logp(12) } else { true };
                    if redundancy {
                        celt_to_silk = dec.bit_logp(1);
                        redundancy_bytes = if mode == Mode::Hybrid {
                            dec.uint(256) as usize + 2
                        } else {
                            len - ((dec.tell() as usize + 7) >> 3)
                        };
                        let new_len = len as i64 - redundancy_bytes as i64;
                        if new_len * 8 < i64::from(dec.tell()) {
                            len = 0;
                            redundancy_bytes = 0;
                            redundancy = false;
                        } else {
                            len = new_len as usize;
                            dec.shrink(len);
                        }
                    }
                }
            }
        }
        if mode != Mode::CeltOnly {
            start_band = 17;
        }
        if redundancy {
            transition = false;
        }
        if transition && mode != Mode::CeltOnly {
            if let Err(e) = self.decode_frame(None, &mut pcm_transition[..f5 * ch], f5.min(audiosize), false) {
                self.silk_pcm = pcm_silk;
                return Err(e);
            }
        }
        if let Some(bw) = bandwidth {
            self.celt.set_end_band(bw.celt_end_band());
        }
        self.celt.set_stream_channels(self.stream_channels);

        let mut redundant_audio = [0f32; 480];
        let mut redundant_rng = 0u32;
        let data_bytes = data.unwrap_or(&[]);
        if redundancy && celt_to_silk {
            self.celt.set_start_band(0);
            let red = &data_bytes[len..len + redundancy_bytes];
            self.celt.decode(Some(red), None, &mut redundant_audio[..f5 * ch], f5)?;
            redundant_rng = self.celt.final_range();
        }
        self.celt.set_start_band(start_band);

        if mode != Mode::SilkOnly {
            let celt_frame_size = f20.min(frame_size);
            if Some(mode) != self.prev_mode && self.prev_mode.is_some() && !self.prev_redundancy {
                self.celt.reset();
            }
            let celt_data = if decode_fec { None } else { data.map(|d| &d[..len]) };
            self.celt.decode(celt_data, range_dec.as_mut(), pcm, celt_frame_size)?;
        } else {
            pcm[..frame_size * ch].fill(0.0);
            if self.prev_mode == Some(Mode::Hybrid) && !(redundancy && celt_to_silk && self.prev_redundancy) {
                self.celt.set_start_band(0);
                let silence = [0xffu8, 0xff];
                self.celt.decode(Some(&silence), None, &mut pcm[..f2_5 * ch], f2_5)?;
            }
        }
        if mode != Mode::CeltOnly {
            for (p, &s) in pcm[..frame_size * ch].iter_mut().zip(&pcm_silk) {
                *p += (1.0 / 32768.0) * f32::from(s);
            }
        }
        self.silk_pcm = pcm_silk;
        let window = celt::window();
        if redundancy && !celt_to_silk {
            self.celt.reset();
            self.celt.set_start_band(0);
            let red = &data_bytes[len..len + redundancy_bytes];
            self.celt.decode(Some(red), None, &mut redundant_audio[..f5 * ch], f5)?;
            redundant_rng = self.celt.final_range();
            let at = ch * (frame_size - f2_5);
            let mut tail = [0f32; 240];
            tail[..f2_5 * ch].copy_from_slice(&pcm[at..at + f2_5 * ch]);
            smooth_fade(&tail, &redundant_audio[ch * f2_5..], &mut pcm[at..], f2_5, ch, window, self.rate);
        }
        if redundancy && celt_to_silk && (self.prev_mode != Some(Mode::SilkOnly) || self.prev_redundancy) {
            pcm[..f2_5 * ch].copy_from_slice(&redundant_audio[..f2_5 * ch]);
            let mut cur = [0f32; 240];
            cur[..f2_5 * ch].copy_from_slice(&pcm[ch * f2_5..ch * 2 * f2_5]);
            smooth_fade(&redundant_audio[ch * f2_5..], &cur, &mut pcm[ch * f2_5..], f2_5, ch, window, self.rate);
        }
        if transition {
            if audiosize >= f5 {
                pcm[..ch * f2_5].copy_from_slice(&pcm_transition[..ch * f2_5]);
                let mut cur = [0f32; 240];
                cur[..f2_5 * ch].copy_from_slice(&pcm[ch * f2_5..ch * 2 * f2_5]);
                smooth_fade(&pcm_transition[ch * f2_5..], &cur, &mut pcm[ch * f2_5..], f2_5, ch, window, self.rate);
            } else {
                let mut cur = [0f32; 240];
                cur[..f2_5 * ch].copy_from_slice(&pcm[..ch * f2_5]);
                smooth_fade(&pcm_transition, &cur, pcm, f2_5, ch, window, self.rate);
            }
        }
        self.range_final = if len <= 1 {
            0
        } else {
            range_dec.as_ref().map_or(0, RangeDecoder::range) ^ redundant_rng
        };
        self.prev_mode = Some(mode);
        self.prev_redundancy = redundancy && !celt_to_silk;
        Ok(audiosize)
    }
}

fn smooth_fade(in1: &[f32], in2: &[f32], out: &mut [f32], overlap: usize, channels: usize, window: &[f32], rate: u32) {
    let inc = (48_000 / rate) as usize;
    for c in 0..channels {
        for i in 0..overlap {
            let w = window[i * inc] * window[i * inc];
            out[i * channels + c] = w * in2[i * channels + c] + (1.0 - w) * in1[i * channels + c];
        }
    }
}

/// `x * 32768` rounded to nearest-even and saturated. Adding and removing
/// 1.5 * 2^23 rounds with the FPU's default nearest-even mode in two plain
/// additions (what `lrintf` does in the reference), avoiding a libm call.
#[inline(always)]
pub fn float_to_i16(x: f32) -> i16 {
    const MAGIC: f32 = 12_582_912.0;
    let v = (x * 32768.0).clamp(-32768.0, 32767.0);
    ((v + MAGIC) - MAGIC) as i16
}

/// Smooth non-linearity keeping float output inside [-1, 1] without hard
/// clipping (same behaviour as the reference `opus_pcm_soft_clip`).
pub fn soft_clip_pcm(x: &mut [f32], n: usize, c: usize, mem: &mut [f32; 2]) {
    if c < 1 || n < 1 {
        return;
    }
    for v in x.iter_mut() {
        *v = v.clamp(-2.0, 2.0);
    }
    for ch in 0..c {
        let mut a = mem[ch];
        for i in 0..n {
            let v = x[i * c + ch];
            if v * a >= 0.0 {
                break;
            }
            x[i * c + ch] = v + a * v * v;
        }
        let mut curr = 0;
        let x0 = x[ch];
        loop {
            let mut i = curr;
            while i < n {
                let v = x[i * c + ch];
                if v > 1.0 || v < -1.0 {
                    break;
                }
                i += 1;
            }
            if i == n {
                a = 0.0;
                break;
            }
            let mut peak_pos = i;
            let mut start = i;
            let mut end = i;
            let xi = x[i * c + ch];
            let mut maxval = xi.abs();
            while start > 0 && xi * x[(start - 1) * c + ch] >= 0.0 {
                start -= 1;
            }
            while end < n && xi * x[end * c + ch] >= 0.0 {
                let v = x[end * c + ch].abs();
                if v > maxval {
                    maxval = v;
                    peak_pos = end;
                }
                end += 1;
            }
            let special = start == 0 && xi * x[ch] >= 0.0;
            a = (maxval - 1.0) / (maxval * maxval);
            a += a * 2.4e-7;
            if xi > 0.0 {
                a = -a;
            }
            for k in start..end {
                let v = x[k * c + ch];
                x[k * c + ch] = v + a * v * v;
            }
            if special && peak_pos >= 2 {
                let mut offset = x0 - x[ch];
                let delta = offset / peak_pos as f32;
                for k in curr..peak_pos {
                    offset -= delta;
                    let v = x[k * c + ch] + offset;
                    x[k * c + ch] = v.clamp(-1.0, 1.0);
                }
            }
            curr = end;
            if curr == n {
                break;
            }
        }
        mem[ch] = a;
    }
}
