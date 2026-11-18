//! Packet framing (RFC 6716 section 3): TOC byte, frame counts and sizes.

use crate::Error;

/// Coding mode selected by the TOC configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    SilkOnly,
    Hybrid,
    CeltOnly,
}

/// Audio bandwidth (RFC 6716 table 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bandwidth {
    Narrow,
    Medium,
    Wide,
    SuperWide,
    Full,
}

impl Bandwidth {
    /// Last coded CELT band + 1 for this bandwidth.
    pub fn celt_end_band(self) -> usize {
        match self {
            Self::Narrow => 13,
            Self::Medium | Self::Wide => 17,
            Self::SuperWide => 19,
            Self::Full => 21,
        }
    }
}

/// Decoded table-of-contents byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toc {
    pub mode: Mode,
    pub bandwidth: Bandwidth,
    /// Frame duration in 48 kHz samples (120..=2880).
    pub frame_size_48k: usize,
    pub stereo: bool,
}

impl Toc {
    pub fn parse(toc: u8) -> Self {
        let config = toc >> 3;
        let (mode, bandwidth, frame_size_48k) = match config {
            0..=11 => {
                let bw = [Bandwidth::Narrow, Bandwidth::Medium, Bandwidth::Wide][usize::from(config / 4)];
                (Mode::SilkOnly, bw, [480, 960, 1920, 2880][usize::from(config & 3)])
            }
            12..=15 => {
                let bw = if config < 14 { Bandwidth::SuperWide } else { Bandwidth::Full };
                (Mode::Hybrid, bw, [480, 960][usize::from(config & 1)])
            }
            _ => {
                let bw = match config {
                    16..=19 => Bandwidth::Narrow,
                    20..=23 => Bandwidth::Wide,
                    24..=27 => Bandwidth::SuperWide,
                    _ => Bandwidth::Full,
                };
                (Mode::CeltOnly, bw, 120 << (config & 3))
            }
        };
        Self {
            mode,
            bandwidth,
            frame_size_48k,
            stereo: toc & 4 != 0,
        }
    }

    /// Frame duration at `rate` Hz.
    pub fn frame_size(self, rate: u32) -> usize {
        self.frame_size_48k * rate as usize / 48_000
    }
}

/// Frame byte ranges of a packet without heap allocation (at most 48
/// frames of 2.5 ms make up 120 ms).
#[derive(Clone, Debug)]
pub struct Frames {
    start: [u16; 48],
    len: [u16; 48],
    count: usize,
}

impl Frames {
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn get(&self, i: usize) -> core::ops::Range<usize> {
        let s = usize::from(self.start[i]);
        s..s + usize::from(self.len[i])
    }
    pub fn iter(&self) -> impl Iterator<Item = core::ops::Range<usize>> + '_ {
        (0..self.count).map(|i| self.get(i))
    }
}

impl PartialEq<Vec<core::ops::Range<usize>>> for Frames {
    fn eq(&self, other: &Vec<core::ops::Range<usize>>) -> bool {
        self.iter().eq(other.iter().cloned())
    }
}

/// A parsed packet: TOC, frame byte ranges, and total bytes consumed
/// (including padding; relevant for self-delimited multistream packets).
#[derive(Clone, Debug)]
pub struct Packet {
    pub toc: Toc,
    pub frames: Frames,
    pub consumed: usize,
}

fn parse_size(data: &[u8]) -> Option<(usize, usize)> {
    match data {
        [] => None,
        [b, ..] if *b < 252 => Some((usize::from(*b), 1)),
        [b0, b1, ..] => Some((4 * usize::from(*b1) + usize::from(*b0), 2)),
        _ => None,
    }
}

impl Packet {
    /// Split `data` into frames. `self_delimited` is the multistream framing
    /// of RFC 6716 appendix B.
    pub fn parse(data: &[u8], self_delimited: bool) -> Result<Self, Error> {
        let invalid = Error::InvalidPacket;
        let (&toc_byte, _) = data.split_first().ok_or(invalid)?;
        let toc = Toc::parse(toc_byte);
        let mut pos = 1usize;
        let mut len = data.len() - 1;
        let mut sizes = [0usize; 48];
        let mut last_size = len;
        let mut cbr = false;
        let count;
        let mut pad = 0usize;
        match toc_byte & 3 {
            0 => count = 1,
            1 => {
                count = 2;
                cbr = true;
                if !self_delimited {
                    if len & 1 != 0 {
                        return Err(invalid);
                    }
                    last_size = len / 2;
                    sizes[0] = last_size;
                }
            }
            2 => {
                count = 2;
                let (size, bytes) = parse_size(&data[pos..]).ok_or(invalid)?;
                len -= bytes;
                if size > len {
                    return Err(invalid);
                }
                pos += bytes;
                sizes[0] = size;
                last_size = len - size;
            }
            _ => {
                if len < 1 {
                    return Err(invalid);
                }
                let ch = data[pos];
                pos += 1;
                count = usize::from(ch & 0x3f);
                if count == 0 || toc.frame_size_48k * count > 5760 {
                    return Err(invalid);
                }
                let mut l = len as isize - 1;
                if ch & 0x40 != 0 {
                    loop {
                        if l <= 0 {
                            return Err(invalid);
                        }
                        let p = data[pos];
                        pos += 1;
                        l -= 1;
                        let tmp = if p == 255 { 254 } else { usize::from(p) };
                        l -= tmp as isize;
                        pad += tmp;
                        if p != 255 {
                            break;
                        }
                    }
                }
                if l < 0 {
                    return Err(invalid);
                }
                len = l as usize;
                cbr = ch & 0x80 == 0;
                if !cbr {
                    let mut last = len as isize;
                    for size in sizes.iter_mut().take(count - 1) {
                        let (s, bytes) = parse_size(&data[pos..]).ok_or(invalid)?;
                        len = len.checked_sub(bytes).ok_or(invalid)?;
                        if s > len {
                            return Err(invalid);
                        }
                        pos += bytes;
                        *size = s;
                        last -= (bytes + s) as isize;
                    }
                    if last < 0 {
                        return Err(invalid);
                    }
                    last_size = last as usize;
                } else if !self_delimited {
                    last_size = len / count;
                    if last_size * count != len {
                        return Err(invalid);
                    }
                    sizes[..count - 1].fill(last_size);
                }
            }
        }
        if self_delimited {
            let (size, bytes) = parse_size(&data[pos..]).ok_or(invalid)?;
            len = len.checked_sub(bytes).ok_or(invalid)?;
            if size > len {
                return Err(invalid);
            }
            pos += bytes;
            sizes[count - 1] = size;
            if cbr {
                if size * count > len {
                    return Err(invalid);
                }
                sizes[..count - 1].fill(size);
            } else if bytes + size > last_size {
                return Err(invalid);
            }
        } else {
            if last_size > 1275 {
                return Err(invalid);
            }
            sizes[count - 1] = last_size;
        }
        let mut frames = Frames {
            start: [0; 48],
            len: [0; 48],
            count,
        };
        for (i, &size) in sizes[..count].iter().enumerate() {
            if size > 1275 || pos + size > data.len() || pos > usize::from(u16::MAX) {
                return Err(invalid);
            }
            frames.start[i] = pos as u16;
            frames.len[i] = size as u16;
            pos += size;
        }
        Ok(Self {
            toc,
            frames,
            consumed: pos + pad,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_configurations() {
        let t = Toc::parse(0x00);
        assert_eq!((t.mode, t.bandwidth, t.frame_size_48k), (Mode::SilkOnly, Bandwidth::Narrow, 480));
        let t = Toc::parse(15 << 3 | 4);
        assert_eq!((t.mode, t.bandwidth, t.frame_size_48k, t.stereo), (Mode::Hybrid, Bandwidth::Full, 960, true));
        let t = Toc::parse(31 << 3);
        assert_eq!((t.mode, t.bandwidth, t.frame_size_48k), (Mode::CeltOnly, Bandwidth::Full, 960));
        let t = Toc::parse(16 << 3);
        assert_eq!(t.frame_size_48k, 120);
    }

    #[test]
    fn framing_codes() {
        let p = Packet::parse(&[0xf8, 1, 2, 3], false).unwrap();
        assert_eq!(p.frames, vec![1..4]);
        let p = Packet::parse(&[0xf9, 1, 2, 3, 4], false).unwrap();
        assert_eq!(p.frames, vec![1..3, 3..5]);
        assert!(Packet::parse(&[0xf9, 1, 2, 3], false).is_err());
        let p = Packet::parse(&[0xfa, 1, 9, 8, 7], false).unwrap();
        assert_eq!(p.frames, vec![2..3, 3..5]);
        // Code 3, CBR, 3 frames, 2 bytes padding.
        let p = Packet::parse(&[0xfb, 0x43, 2, 1, 2, 3, 0, 0], false).unwrap();
        assert_eq!(p.frames, vec![3..4, 4..5, 5..6]);
        assert_eq!(p.consumed, 8);
        // CBR payload not divisible by the frame count.
        assert!(Packet::parse(&[0xfb, 0x43, 2, 1, 1, 2, 3, 0, 0], false).is_err());
        // Code 3 VBR: sizes 1 and 2, last gets the rest.
        let p = Packet::parse(&[0xfb, 0x83, 1, 2, 9, 8, 8, 7, 7], false).unwrap();
        assert_eq!(p.frames, vec![4..5, 5..7, 7..9]);
        assert!(Packet::parse(&[], false).is_err());
        assert!(Packet::parse(&[0xfb, 0x00], false).is_err());
    }
}
