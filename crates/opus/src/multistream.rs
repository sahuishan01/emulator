//! Multistream decoding (RFC 7845 section 5.1.1 channel mapping): several
//! elementary Opus streams, all but the last self-delimited, mixed into one
//! interleaved output.

use crate::decoder::{float_to_i16, Decoder};
use crate::packet::Packet;
use crate::Error;

/// A multistream decoder: `coupled` stereo streams followed by mono
/// streams, routed to output channels by `mapping` (255 = silent).
#[derive(Clone)]
pub struct MultiStreamDecoder {
    rate: u32,
    channels: usize,
    coupled: usize,
    mapping: Vec<u8>,
    decoders: Vec<Decoder>,
    buf: Vec<f32>,
}

impl MultiStreamDecoder {
    pub fn new(rate: u32, channels: usize, streams: usize, coupled: usize, mapping: &[u8]) -> Result<Self, Error> {
        if channels == 0
            || channels > 255
            || streams == 0
            || coupled > streams
            || streams + coupled > 255
            || mapping.len() < channels
            || mapping[..channels].iter().any(|&m| m != 255 && usize::from(m) >= streams + coupled)
        {
            return Err(Error::BadArgument);
        }
        let decoders = (0..streams)
            .map(|s| Decoder::new(rate, if s < coupled { 2 } else { 1 }))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            rate,
            channels,
            coupled,
            mapping: mapping[..channels].to_vec(),
            decoders,
            buf: Vec::new(),
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn streams(&self) -> usize {
        self.decoders.len()
    }

    pub fn reset(&mut self) {
        for d in &mut self.decoders {
            d.reset();
        }
    }

    /// Final ranges XORed over all streams (as the reference reports them).
    pub fn final_range(&self) -> u32 {
        self.decoders.iter().fold(0, |acc, d| acc ^ d.final_range())
    }

    fn validate(&self, mut data: &[u8]) -> Result<usize, Error> {
        let streams = self.decoders.len();
        let mut samples = 0;
        for s in 0..streams {
            if data.is_empty() {
                return Err(Error::InvalidPacket);
            }
            let p = Packet::parse(data, s != streams - 1)?;
            let n = p.frames.len() * p.toc.frame_size(self.rate);
            if s != 0 && n != samples {
                return Err(Error::InvalidPacket);
            }
            samples = n;
            data = &data[p.consumed.min(data.len())..];
        }
        Ok(samples)
    }

    /// Decode into interleaved 16-bit PCM (`channels` per sample).
    pub fn decode(&mut self, packet: Option<&[u8]>, pcm: &mut [i16], frame_size: usize) -> Result<usize, Error> {
        let mut frame_size = frame_size.min(self.rate as usize / 25 * 3).min(pcm.len() / self.channels);
        if frame_size == 0 {
            return Err(Error::BadArgument);
        }
        let data = packet.unwrap_or(&[]);
        let plc = data.is_empty();
        let streams = self.decoders.len();
        if !plc {
            if data.len() < 2 * streams - 1 {
                return Err(Error::InvalidPacket);
            }
            if self.validate(data)? > frame_size {
                return Err(Error::BufferTooSmall);
            }
        }
        let mut buf = std::mem::take(&mut self.buf);
        buf.resize(2 * frame_size, 0.0);
        let mut rest = data;
        for s in 0..streams {
            let (n, consumed) = self.decoders[s].decode_stream(rest, s != streams - 1, &mut buf, frame_size)?;
            if !plc {
                rest = &rest[consumed.min(rest.len())..];
            }
            frame_size = n;
            let nc = self.channels;
            if s < self.coupled {
                for (c, &m) in self.mapping.iter().enumerate() {
                    let src = if usize::from(m) == 2 * s {
                        0
                    } else if usize::from(m) == 2 * s + 1 {
                        1
                    } else {
                        continue;
                    };
                    for i in 0..n {
                        pcm[i * nc + c] = float_to_i16(buf[2 * i + src]);
                    }
                }
            } else {
                for (c, &m) in self.mapping.iter().enumerate() {
                    if usize::from(m) == s + self.coupled {
                        for i in 0..n {
                            pcm[i * nc + c] = float_to_i16(buf[i]);
                        }
                    }
                }
            }
        }
        for (c, &m) in self.mapping.iter().enumerate() {
            if m == 255 {
                for i in 0..frame_size {
                    pcm[i * self.channels + c] = 0;
                }
            }
        }
        self.buf = buf;
        Ok(frame_size)
    }
}
