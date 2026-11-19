//! Structured Maxwell instruction decoder (fields only, no semantics).
//!
//! Turns 64-bit instruction words into typed operands so the translator can
//! work on fields instead of raw bit masks. Field positions are written from
//! memory of the public instruction-encoding descriptions and are unverified
//! until real programs translate and render correctly. Anything outside the
//! listed forms is `Unsupported`; nothing is guessed or skipped.

/// Predicate register 7 is PT (always true).
pub const PT: u8 = 7;
/// Register 255 is RZ (reads zero, writes discarded).
pub const RZ: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pred {
    pub index: u8,
    pub negate: bool,
}
impl Pred {
    fn at(word: u64, bit: u32) -> Self {
        Pred {
            index: field(word, bit, 3) as u8,
            negate: field(word, bit + 3, 1) != 0,
        }
    }
    pub fn is_always(self) -> bool {
        self.index == PT && !self.negate
    }
}

/// Second ALU source (operand B), selected by the encoding form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Reg(u8),
    /// c[bank][offset], offset in bytes.
    Cbuf {
        bank: u8,
        offset: u32,
    },
    /// 20-bit immediate: bits 0..18 from the word, bit 19 is the sign bit
    /// (word bit 56). Float forms use it as the top 20 bits of an f32.
    Imm20(u32),
}
impl Source {
    /// The f32 bit pattern of a float-form immediate.
    pub fn imm_f32_bits(imm: u32) -> u32 {
        imm << 12
    }
    /// Sign-extended integer immediate.
    pub fn imm_i32(imm: u32) -> i32 {
        ((imm << 12) as i32) >> 12
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MufuOp {
    Cos,
    Sin,
    Ex2,
    Lg2,
    Rcp,
    Rsq,
    Rcp64h,
    Rsq64h,
    Sqrt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmadMode {
    /// c used as is.
    Full,
    /// c & 0xffff.
    Lo,
    /// c >> 16.
    Hi,
    /// Mode 3 (CSFU); semantics not modelled.
    Csfu,
    /// c + (b << 16).
    Bcc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instr {
    Nop,
    /// Condition code test in bits 0..4 (0xf = always).
    Exit {
        cc: u8,
    },
    /// Target is a byte offset relative to the next instruction.
    Bra {
        cc: u8,
        offset: i32,
    },
    Ssy {
        offset: i32,
    },
    Sync {
        cc: u8,
    },
    Mov {
        d: u8,
        b: Source,
        lanes: u8,
    },
    Mov32i {
        d: u8,
        imm: u32,
        lanes: u8,
    },
    Fadd {
        d: u8,
        a: u8,
        b: Source,
        neg_a: bool,
        abs_a: bool,
        neg_b: bool,
        abs_b: bool,
        sat: bool,
    },
    Fmul {
        d: u8,
        a: u8,
        b: Source,
        neg_b: bool,
        sat: bool,
        scale: u8,
    },
    /// d = a * b + c.
    Ffma {
        d: u8,
        a: u8,
        b: Source,
        c: Source,
        neg_b: bool,
        neg_c: bool,
        sat: bool,
    },
    /// min when `pred` is true, max otherwise.
    Fmnmx {
        d: u8,
        a: u8,
        b: Source,
        pred: Pred,
        neg_a: bool,
        abs_a: bool,
        neg_b: bool,
        abs_b: bool,
    },
    /// p = (a cmp b) bop c; q = !(a cmp b) bop c.
    Fsetp {
        p: u8,
        q: u8,
        a: u8,
        b: Source,
        cmp: u8,
        bop: u8,
        c: Pred,
        neg_a: bool,
        abs_a: bool,
        neg_b: bool,
        abs_b: bool,
    },
    /// p = (a bop1 b) bop2 c; q = (!(a bop1 b)) bop2 c.
    Psetp {
        p: u8,
        q: u8,
        a: Pred,
        b: Pred,
        c: Pred,
        bop1: u8,
        bop2: u8,
    },
    Mufu {
        d: u8,
        a: u8,
        op: MufuOp,
        neg: bool,
        abs: bool,
        sat: bool,
    },
    /// Interpolated attribute read; mode 0 pass, 1 multiply, 2 constant, 3 sc.
    Ipa {
        d: u8,
        attr: u16,
        index: u8,
        multiplier: u8,
        mode: u8,
        sample: u8,
        sat: bool,
    },
    /// 16x16 multiply-add: d = (a.half * b.half) [<< 16] + mode(c) [merged].
    Xmad {
        d: u8,
        a: u8,
        /// Immediate forms carry a plain 16-bit value.
        b: Source,
        c: u8,
        sign_a: bool,
        sign_b: bool,
        high_a: bool,
        high_b: bool,
        /// Product shifted left by 16.
        psl: bool,
        /// Result low half merged with operand B's low half in the high half.
        mrg: bool,
        /// Extended (carry-in) add.
        x: bool,
        /// Writes the condition code.
        cc: bool,
        mode: XmadMode,
    },
    /// Texture fetch with scalar register layout; all fields raw.
    Texs {
        d0: u8,
        d1: u8,
        a: u8,
        b: u8,
        handle: u16,
        kind: u8,
        mask: u8,
    },
    /// d = a * imm (full 32-bit float immediate).
    Fmul32i {
        d: u8,
        a: u8,
        imm: u32,
        sat: bool,
    },
    /// Integer add; `x` adds the carry flag.
    Iadd {
        d: u8,
        a: u8,
        b: Source,
        neg_a: bool,
        neg_b: bool,
        sat: bool,
        x: bool,
    },
    /// d = (c cmp 0.0) ? a : b.
    Fcmp {
        d: u8,
        a: u8,
        b: Source,
        c: Source,
        cmp: u8,
    },
    /// d = ((a cmp b) bop c) ? (float_result ? 1.0 : !0) : 0.
    Fset {
        d: u8,
        a: u8,
        b: Source,
        cmp: u8,
        bop: u8,
        c: Pred,
        neg_a: bool,
        abs_a: bool,
        neg_b: bool,
        abs_b: bool,
        float_result: bool,
    },
    /// Other texture forms: decoded only as "a texture instruction".
    TexOther {
        top: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub guard: Pred,
    pub instr: Instr,
}

fn field(word: u64, bit: u32, width: u32) -> u64 {
    (word >> bit) & ((1u64 << width) - 1)
}
fn bit(word: u64, at: u32) -> bool {
    field(word, at, 1) != 0
}
fn reg(word: u64, at: u32) -> u8 {
    field(word, at, 8) as u8
}
fn imm20(word: u64) -> u32 {
    field(word, 20, 19) as u32 | (field(word, 56, 1) as u32) << 19
}
fn cbuf(word: u64) -> Source {
    Source::Cbuf {
        bank: field(word, 34, 5) as u8,
        offset: (field(word, 20, 14) as u32) * 4,
    }
}
fn branch_offset(word: u64) -> i32 {
    ((field(word, 20, 24) as u32) << 8) as i32 >> 8
}

/// ALU source B for the 0x5c / 0x4c / 0x38 family (register, cbuf, immediate).
fn alu_b(word: u64) -> Option<(u16, Source)> {
    let top = (word >> 48) as u16;
    match top >> 8 {
        0x5c => Some((top & 0xfff8, Source::Reg(reg(word, 20)))),
        0x4c => Some((top & 0xfff8 | 0x1000, cbuf(word))),
        0x38 | 0x39 => Some(((top & 0xfef8) + 0x2400, Source::Imm20(imm20(word)))),
        _ => None,
    }
}

/// Decode one instruction; None when the form is not listed.
pub fn decode_word(word: u64) -> Option<Decoded> {
    let guard = Pred::at(word, 16);
    let top = (word >> 48) as u16;
    let d = reg(word, 0);
    let a = reg(word, 8);
    let cc = field(word, 0, 5) as u8;
    let instr = if top & 0xfff8 == 0x50b0 {
        Instr::Nop
    } else if top & 0xfff0 == 0xe300 {
        Instr::Exit { cc }
    } else if top & 0xfff0 == 0xe240 {
        Instr::Bra {
            cc,
            offset: branch_offset(word),
        }
    } else if top & 0xfff0 == 0xe290 {
        Instr::Ssy {
            offset: branch_offset(word),
        }
    } else if top & 0xfff8 == 0xf0f8 {
        Instr::Sync { cc }
    } else if top & 0xfe00 == 0x1e00 {
        Instr::Fmul32i {
            d,
            a,
            imm: field(word, 20, 32) as u32,
            sat: bit(word, 55),
        }
    } else if matches!(top & 0xfff0, 0x5ba0 | 0x4ba0 | 0x53a0) || top & 0xfef0 == 0x36a0 {
        let rc = Source::Reg(reg(word, 39));
        let (b, c) = match top >> 8 {
            0x5b => (Source::Reg(reg(word, 20)), rc),
            0x4b => (cbuf(word), rc),
            0x53 => (Source::Reg(reg(word, 39)), cbuf(word)),
            _ => (Source::Imm20(imm20(word)), rc),
        };
        Instr::Fcmp {
            d,
            a,
            b,
            c,
            cmp: field(word, 48, 4) as u8,
        }
    } else if matches!(top & 0xff00, 0x5800 | 0x4800) || top & 0xfe00 == 0x3000 {
        let b = match top >> 8 {
            0x58 => Source::Reg(reg(word, 20)),
            0x48 => cbuf(word),
            _ => Source::Imm20(imm20(word)),
        };
        Instr::Fset {
            d,
            a,
            b,
            c: Pred::at(word, 39),
            neg_a: bit(word, 43),
            abs_b: bit(word, 44),
            bop: field(word, 45, 2) as u8,
            cmp: field(word, 48, 4) as u8,
            float_result: bit(word, 52),
            neg_b: bit(word, 53),
            abs_a: bit(word, 54),
        }
    } else if top & 0xfff0 == 0x0100 {
        Instr::Mov32i {
            d,
            imm: field(word, 20, 32) as u32,
            lanes: field(word, 12, 4) as u8,
        }
    } else if let Some((base, b)) = alu_b(word) {
        match base {
            0x5c98 => Instr::Mov {
                d,
                b,
                lanes: field(word, 39, 4) as u8,
            },
            0x5c58 => Instr::Fadd {
                d,
                a,
                b,
                neg_b: bit(word, 45),
                abs_a: bit(word, 46),
                neg_a: bit(word, 48),
                abs_b: bit(word, 49),
                sat: bit(word, 50),
            },
            0x5c68 => Instr::Fmul {
                d,
                a,
                b,
                scale: field(word, 41, 3) as u8,
                neg_b: bit(word, 48),
                sat: bit(word, 50),
            },
            0x5c10 => Instr::Iadd {
                d,
                a,
                b,
                x: bit(word, 43),
                neg_b: bit(word, 48),
                neg_a: bit(word, 49),
                sat: bit(word, 50),
            },
            0x5c60 => Instr::Fmnmx {
                d,
                a,
                b,
                pred: Pred::at(word, 39),
                neg_b: bit(word, 45),
                abs_a: bit(word, 46),
                neg_a: bit(word, 48),
                abs_b: bit(word, 49),
            },
            _ => return None,
        }
    } else if matches!(top & 0xff80, 0x5980 | 0x4980 | 0x5180) || top & 0xfe80 == 0x3280 {
        let rc = Source::Reg(reg(word, 39));
        let (b, c) = match top >> 8 {
            0x59 => (Source::Reg(reg(word, 20)), rc),
            0x49 => (cbuf(word), rc),
            0x51 => (Source::Reg(reg(word, 39)), cbuf(word)),
            _ => (Source::Imm20(imm20(word)), rc),
        };
        Instr::Ffma {
            d,
            a,
            b,
            c,
            neg_b: bit(word, 48),
            neg_c: bit(word, 49),
            sat: bit(word, 50),
        }
    } else if matches!(top & 0xfff0, 0x5bb0 | 0x4bb0) || top & 0xfef0 == 0x36b0 {
        let b = match top >> 8 {
            0x5b => Source::Reg(reg(word, 20)),
            0x4b => cbuf(word),
            _ => Source::Imm20(imm20(word)),
        };
        Instr::Fsetp {
            p: field(word, 3, 3) as u8,
            q: field(word, 0, 3) as u8,
            a,
            b,
            neg_b: bit(word, 6),
            abs_a: bit(word, 7),
            c: Pred::at(word, 39),
            neg_a: bit(word, 43),
            abs_b: bit(word, 44),
            bop: field(word, 45, 2) as u8,
            cmp: field(word, 48, 4) as u8,
        }
    } else if top & 0xfff8 == 0x5090 {
        Instr::Psetp {
            p: field(word, 3, 3) as u8,
            q: field(word, 0, 3) as u8,
            a: Pred::at(word, 12),
            bop1: field(word, 24, 2) as u8,
            b: Pred::at(word, 29),
            c: Pred::at(word, 39),
            bop2: field(word, 45, 2) as u8,
        }
    } else if top & 0xfff8 == 0x5080 {
        let op = match field(word, 20, 4) {
            0 => MufuOp::Cos,
            1 => MufuOp::Sin,
            2 => MufuOp::Ex2,
            3 => MufuOp::Lg2,
            4 => MufuOp::Rcp,
            5 => MufuOp::Rsq,
            6 => MufuOp::Rcp64h,
            7 => MufuOp::Rsq64h,
            8 => MufuOp::Sqrt,
            _ => return None,
        };
        Instr::Mufu {
            d,
            a,
            op,
            abs: bit(word, 46),
            neg: bit(word, 48),
            sat: bit(word, 50),
        }
    } else if top & 0xffc0 == 0xe000 {
        Instr::Ipa {
            d,
            index: a,
            multiplier: reg(word, 20),
            attr: field(word, 28, 10) as u16,
            sat: bit(word, 51),
            sample: field(word, 52, 2) as u8,
            mode: field(word, 54, 2) as u8,
        }
    } else if top & 0xff80 == 0x5b00 || top & 0xfe00 == 0x4e00 || top & 0xfec0 == 0x3600 {
        // Register form: 3-bit mode; cbuf and immediate forms: 2-bit mode.
        let (b, high_b, psl, mrg, x, mode) = match top >> 8 {
            0x5b => (
                Source::Reg(reg(word, 20)),
                bit(word, 35),
                bit(word, 36),
                bit(word, 37),
                bit(word, 38),
                field(word, 50, 3),
            ),
            0x4e | 0x4f => (
                cbuf(word),
                bit(word, 52),
                bit(word, 55),
                bit(word, 56),
                bit(word, 54),
                field(word, 50, 2),
            ),
            _ => (
                Source::Imm20(field(word, 20, 16) as u32),
                false,
                bit(word, 36),
                bit(word, 37),
                bit(word, 38),
                field(word, 50, 2),
            ),
        };
        let mode = match mode {
            0 => XmadMode::Full,
            1 => XmadMode::Lo,
            2 => XmadMode::Hi,
            3 => XmadMode::Csfu,
            4 => XmadMode::Bcc,
            _ => return None,
        };
        Instr::Xmad {
            d,
            a,
            b,
            c: reg(word, 39),
            sign_a: bit(word, 48),
            sign_b: bit(word, 49),
            high_a: bit(word, 53),
            high_b,
            psl,
            mrg,
            x,
            cc: bit(word, 47),
            mode,
        }
    } else if top & 0xfe00 == 0xd800 {
        Instr::Texs {
            d0: d,
            a,
            b: reg(word, 20),
            d1: reg(word, 28),
            handle: field(word, 36, 13) as u16,
            mask: field(word, 50, 3) as u8,
            kind: field(word, 53, 4) as u8,
        }
    } else if top & 0xc000 == 0xc000 {
        Instr::TexOther { top }
    } else {
        return None;
    };
    Some(Decoded { guard, instr })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// (byte offset into code, decoded instruction).
    pub instructions: Vec<(usize, Decoded)>,
    /// First undecodable word and its byte offset.
    pub unsupported: Option<(usize, u64)>,
    /// Undecodable words in total (the scan continues past the first).
    pub unsupported_count: usize,
    /// Byte offsets of every undecodable word.
    pub undecoded_offsets: Vec<usize>,
    /// Up to 12 undecodable words with distinct top 16 bits (offset, word).
    pub undecoded_examples: Vec<(usize, u64)>,
    /// All-zero words (not an instruction; counted separately).
    pub zero_words: usize,
}

/// Decode every instruction slot of whole 32-byte bundles up to and including
/// the end-of-program self branch.
pub fn list(code: &[u8]) -> Listing {
    let mut listing = Listing::default();
    'outer: for (n, bundle) in code.chunks_exact(32).enumerate() {
        for (slot, bytes) in bundle[8..].chunks_exact(8).enumerate() {
            let offset = n * 32 + 8 + slot * 8;
            let word = u64::from_le_bytes(bytes.try_into().expect("word"));
            if word == 0 {
                listing.zero_words += 1;
            } else if let Some(decoded) = decode_word(word) {
                listing.instructions.push((offset, decoded));
            } else {
                listing.unsupported_count += 1;
                listing.unsupported.get_or_insert((offset, word));
                listing.undecoded_offsets.push(offset);
                if listing.undecoded_examples.len() < 12
                    && !listing
                        .undecoded_examples
                        .iter()
                        .any(|(_, w)| w >> 48 == word >> 48 && (w >> 48 != 0 || *w == word))
                {
                    listing.undecoded_examples.push((offset, word));
                }
            }
            if word == crate::census::SELF_BRANCH {
                break 'outer;
            }
        }
    }
    listing
}
