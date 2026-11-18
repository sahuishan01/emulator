//! Range decoder (RFC 6716 section 4.1).
//!
//! The decoder keeps a 32-bit window like the reference design: `rng` is the
//! current range, `val` the distance from the top of the range to the coded
//! value. Raw bits are read from the end of the frame in a separate window.

const SYM_BITS: u32 = 8;
const CODE_BITS: u32 = 32;
const SYM_MAX: u32 = (1 << SYM_BITS) - 1;
const CODE_TOP: u32 = 1 << (CODE_BITS - 1);
const CODE_BOT: u32 = CODE_TOP >> SYM_BITS;
const CODE_EXTRA: u32 = (CODE_BITS - 2) % SYM_BITS + 1;
const UINT_BITS: u32 = 8;
const WINDOW_SIZE: u32 = 32;

/// Fractional bit resolution used by `tell_frac` (1/8 bit).
pub const BITRES: u32 = 3;

/// Number of bits needed to represent `x` (0 for 0).
#[inline]
pub fn ilog(x: u32) -> u32 {
    32 - x.leading_zeros()
}

pub struct RangeDecoder<'a> {
    buf: &'a [u8],
    /// Bytes of `buf` that belong to this coder (may be shrunk by CELT).
    storage: usize,
    offs: usize,
    end_offs: usize,
    end_window: u32,
    nend_bits: u32,
    nbits_total: i32,
    rng: u32,
    val: u32,
    ext: u32,
    rem: u32,
    error: bool,
}

impl<'a> RangeDecoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        let mut d = Self {
            buf,
            storage: buf.len(),
            offs: 0,
            end_offs: 0,
            end_window: 0,
            nend_bits: 0,
            nbits_total: (CODE_BITS + 1 - ((CODE_BITS - CODE_EXTRA) / SYM_BITS) * SYM_BITS) as i32,
            rng: 1 << CODE_EXTRA,
            val: 0,
            ext: 0,
            rem: 0,
            error: false,
        };
        d.rem = d.read_byte();
        d.val = d.rng - 1 - (d.rem >> (SYM_BITS - CODE_EXTRA));
        d.normalize();
        d
    }

    #[inline]
    fn read_byte(&mut self) -> u32 {
        if self.offs < self.storage {
            let b = self.buf[self.offs];
            self.offs += 1;
            u32::from(b)
        } else {
            0
        }
    }

    #[inline]
    fn read_byte_from_end(&mut self) -> u32 {
        if self.end_offs < self.storage {
            self.end_offs += 1;
            u32::from(self.buf[self.storage - self.end_offs])
        } else {
            0
        }
    }

    #[inline]
    fn normalize(&mut self) {
        while self.rng <= CODE_BOT {
            self.nbits_total += SYM_BITS as i32;
            self.rng <<= SYM_BITS;
            let mut sym = self.rem;
            self.rem = self.read_byte();
            sym = ((sym << SYM_BITS) | self.rem) >> (SYM_BITS - CODE_EXTRA);
            self.val = ((self.val << SYM_BITS).wrapping_add(SYM_MAX & !sym)) & (CODE_TOP - 1);
        }
    }

    /// First half of a symbol decode with total frequency `ft`.
    #[inline]
    pub fn decode(&mut self, ft: u32) -> u32 {
        self.ext = self.rng / ft;
        let s = self.val / self.ext;
        ft - (s + 1).min(ft)
    }

    /// `decode` with `ft = 1 << bits`.
    #[inline]
    pub fn decode_bin(&mut self, bits: u32) -> u32 {
        self.ext = self.rng >> bits;
        let s = self.val / self.ext;
        (1 << bits) - (s + 1).min(1 << bits)
    }

    /// Second half of a symbol decode: consume `[fl, fh)` of `ft`.
    #[inline]
    pub fn update(&mut self, fl: u32, fh: u32, ft: u32) {
        let s = self.ext.wrapping_mul(ft - fh);
        self.val = self.val.wrapping_sub(s);
        self.rng = if fl > 0 {
            self.ext.wrapping_mul(fh - fl)
        } else {
            self.rng - s
        };
        self.normalize();
    }

    /// One bit whose probability of being 1 is `1 / (1 << logp)`.
    #[inline]
    pub fn bit_logp(&mut self, logp: u32) -> bool {
        let r = self.rng;
        let d = self.val;
        let s = r >> logp;
        let ret = d < s;
        if !ret {
            self.val = d - s;
        }
        self.rng = if ret { s } else { r - s };
        self.normalize();
        ret
    }

    /// Symbol from an inverse CDF table with total `1 << ftb`.
    #[inline]
    pub fn icdf(&mut self, icdf: &[u8], ftb: u32) -> usize {
        let mut s = self.rng;
        let d = self.val;
        let r = s >> ftb;
        let mut ret = 0;
        let mut t;
        loop {
            t = s;
            s = r.wrapping_mul(u32::from(icdf[ret]));
            if d >= s {
                break;
            }
            ret += 1;
        }
        self.val = d - s;
        self.rng = t - s;
        self.normalize();
        ret
    }

    /// Uniform integer in `[0, ft)`, `ft > 1`.
    pub fn uint(&mut self, ft: u32) -> u32 {
        debug_assert!(ft > 1);
        let ft = ft - 1;
        let mut ftb = ilog(ft);
        if ftb > UINT_BITS {
            ftb -= UINT_BITS;
            let ft1 = (ft >> ftb) + 1;
            let s = self.decode(ft1);
            self.update(s, s + 1, ft1);
            let t = (s << ftb) | self.bits(ftb);
            if t <= ft {
                t
            } else {
                self.error = true;
                ft
            }
        } else {
            let ft = ft + 1;
            let s = self.decode(ft);
            self.update(s, s + 1, ft);
            s
        }
    }

    /// Raw bits from the end of the frame (`bits <= 25`).
    pub fn bits(&mut self, bits: u32) -> u32 {
        let mut window = self.end_window;
        let mut available = self.nend_bits;
        if available < bits {
            loop {
                window |= self.read_byte_from_end() << available;
                available += SYM_BITS;
                if available > WINDOW_SIZE - SYM_BITS {
                    break;
                }
            }
        }
        let ret = window & ((1u32 << bits) - 1);
        self.end_window = if bits == 32 { 0 } else { window >> bits };
        self.nend_bits = available - bits;
        self.nbits_total += bits as i32;
        ret
    }

    /// Laplace-distributed value (CELT coarse energy), RFC 6716 4.3.2.1.
    pub fn laplace(&mut self, fs: u32, decay: u32) -> i32 {
        const MINP: u32 = 1;
        const NMIN: u32 = 16;
        let mut val = 0i32;
        let fm = self.decode_bin(15);
        let mut fl = 0u32;
        let mut fs = fs;
        if fm >= fs {
            val += 1;
            fl = fs;
            fs = (((32768 - MINP * (2 * NMIN) - fs) * (16384 - decay)) >> 15) + MINP;
            while fs > MINP && fm >= fl + 2 * fs {
                fs *= 2;
                fl += fs;
                fs = ((fs - 2 * MINP) * decay) >> 15;
                fs += MINP;
                val += 1;
            }
            if fs <= MINP {
                let di = (fm - fl) >> 1;
                val += di as i32;
                fl += 2 * di * MINP;
            }
            if fm < fl + fs {
                val = -val;
            } else {
                fl += fs;
            }
        }
        self.update(fl, (fl + fs).min(32768), 32768);
        val
    }

    /// Whole bits consumed so far (rounded up).
    #[inline]
    pub fn tell(&self) -> i32 {
        self.nbits_total - ilog(self.rng) as i32
    }

    /// Bits consumed so far in 1/8 bit units.
    pub fn tell_frac(&self) -> u32 {
        const CORRECTION: [u32; 8] = [35733, 38967, 42495, 46340, 50535, 55109, 60097, 65535];
        let nbits = (self.nbits_total as u32) << BITRES;
        let l = ilog(self.rng);
        let r = self.rng >> (l - 16);
        let mut b = (r >> 12) - 8;
        if r > CORRECTION[b as usize] {
            b += 1;
        }
        nbits - ((l << 3) + b)
    }

    /// Pretend all bits up to `bits` have been read (CELT silence flag).
    pub fn skip_to_bits(&mut self, bits: i32) {
        self.nbits_total += bits - self.tell();
    }

    pub fn range(&self) -> u32 {
        self.rng
    }

    pub fn storage(&self) -> usize {
        self.storage
    }

    /// Shrink the coder to its first `bytes` bytes (CELT redundancy split).
    pub fn shrink(&mut self, bytes: usize) {
        self.storage = bytes.min(self.storage);
    }

    pub fn error(&self) -> bool {
        self.error
    }

    pub fn set_error(&mut self) {
        self.error = true;
    }

    /// Bits of the frame still unread (whole bits).
    pub fn remaining_bits(&self) -> i32 {
        (self.storage as i32) * 8 - self.tell()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_decoder_reports_one_bit() {
        let d = RangeDecoder::new(&[0x80, 0, 0, 0]);
        assert_eq!(d.tell(), 1);
        assert_eq!(d.tell_frac(), 8);
    }

    #[test]
    fn raw_bits_come_from_the_end() {
        let mut d = RangeDecoder::new(&[0, 0, 0xa5, 0x3c]);
        assert_eq!(d.bits(4), 0xc);
        assert_eq!(d.bits(4), 0x3);
        assert_eq!(d.bits(8), 0xa5);
    }
}
