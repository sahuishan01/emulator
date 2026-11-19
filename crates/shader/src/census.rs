//! Opcode census for scoping the shader recompiler (diagnostics only).
//!
//! Classifies Maxwell instruction words by their top 16 bits. The table is
//! written from memory of the public instruction-encoding descriptions and is
//! unverified: a name here means "probably this instruction", not decoded
//! semantics. Unmatched words are counted by their top 16 bits so real game
//! shaders show what the table (and later the translator) is missing.
use std::collections::BTreeMap;

/// (mask, value, name) over bits 63..48 of an instruction word.
const TABLE: &[(u16, u16, &str)] = &[
    // Control flow.
    (0xfff0, 0xe300, "EXIT"),
    (0xfff0, 0xe240, "BRA"),
    (0xfff0, 0xe250, "BRX"),
    (0xfff0, 0xe260, "CAL"),
    (0xfff0, 0xe290, "SSY"),
    (0xfff0, 0xe2a0, "PBK"),
    (0xfff0, 0xe320, "RET"),
    (0xfff0, 0xe330, "KIL"),
    (0xfff0, 0xe340, "BRK"),
    (0xfff8, 0xf0f8, "SYNC"),
    (0xfff8, 0xf0f0, "DEPBAR"),
    (0xfff8, 0xf0a8, "BAR"),
    (0xfff8, 0xef98, "MEMBAR"),
    (0xfff8, 0x50b0, "NOP"),
    // Attributes, constants, memory.
    (0xff00, 0xe000, "IPA"),
    (0xfff8, 0xefd8, "ALD"),
    (0xfff8, 0xeff0, "AST"),
    (0xfff8, 0xef90, "LDC"),
    (0xfff8, 0xeed0, "LDG"),
    (0xfff8, 0xeed8, "STG"),
    (0xfff8, 0xef40, "LDL"),
    (0xfff8, 0xef50, "STL"),
    (0xfff8, 0xef48, "LDS"),
    (0xfff8, 0xef58, "STS"),
    (0xfff8, 0xef10, "SHFL"),
    (0xff00, 0xed00, "ATOM"),
    (0xfff8, 0xebf8, "RED"),
    (0xe000, 0x8000, "LD"),
    (0xe000, 0xa000, "ST"),
    (0xfff8, 0xf0c8, "S2R"),
    (0xfff8, 0x50d8, "VOTE"),
    (0xfff8, 0x50f8, "FSWZADD"),
    // Texture family (grouped; sub-forms not distinguished).
    (0xfe00, 0xd800, "TEXS"),
    (0xfe00, 0xda00, "TLDS"),
    (0xff00, 0xdf00, "TLD4S/TXQ/TMML"),
    (0xfe00, 0xdc00, "TLD"),
    (0xc000, 0xc000, "TEX*"),
    // Float.
    (0xff80, 0x5980, "FFMA"),
    (0xff80, 0x4980, "FFMA"),
    (0xff80, 0x5180, "FFMA"),
    (0xfe80, 0x3280, "FFMA"),
    (0xfc00, 0x0c00, "FFMA32I"),
    (0xfc00, 0x0800, "FADD32I"),
    (0xfe00, 0x1e00, "FMUL32I"),
    (0xfff8, 0x5080, "MUFU"),
    (0xfff0, 0x5bb0, "FSETP"),
    (0xfff0, 0x4bb0, "FSETP"),
    (0xfef0, 0x36b0, "FSETP"),
    (0xfff0, 0x5800, "FSET"),
    (0xfff0, 0x4800, "FSET"),
    (0xfe00, 0x3000, "FSET"),
    (0xfff8, 0x5d00, "HFMA2"),
    (0xfff8, 0x5d08, "HMUL2"),
    (0xfff8, 0x5d10, "HADD2"),
    // Integer.
    (0xfe00, 0x1c00, "IADD32I"),
    (0xfc00, 0x1400, "ISCADD32I"),
    (0xff80, 0x5a00, "IMAD"),
    (0xff80, 0x4a00, "IMAD"),
    (0xfe80, 0x3400, "IMAD"),
    (0xff80, 0x5b00, "XMAD"),
    (0xfe00, 0x4e00, "XMAD"),
    (0xfe00, 0x3600, "XMAD"),
    (0xfff0, 0x5b60, "ISETP"),
    (0xfff0, 0x4b60, "ISETP"),
    (0xfef0, 0x3660, "ISETP"),
    (0xfff0, 0x5b50, "ISET"),
    (0xfff0, 0x5b40, "ICMP"),
    (0xfff0, 0x5be0, "LOP3"),
    (0xfc00, 0x3c00, "LOP3"),
    (0xfc00, 0x0400, "LOP32I"),
    (0xfff0, 0x5bf0, "BFI"),
    (0xfff8, 0x5bf8, "SHF"),
    (0xfff0, 0x5bc0, "PRMT"),
    (0xfff0, 0x4bc0, "PRMT"),
    (0xfff0, 0x0100, "MOV32I"),
    (0xfff8, 0x5090, "PSETP"),
    (0xfff8, 0x50a0, "CSETP"),
    (0xfff8, 0x5b70, "DFMA"),
];

/// Register forms 0x5cXX with constant-buffer (0x4cXX) and immediate
/// (0x38XX, bit 56 is the immediate sign) variants.
const ALU_5C: &[(u16, &str)] = &[
    (0x5c00, "BFE"),
    (0x5c08, "POPC"),
    (0x5c10, "IADD"),
    (0x5c18, "ISCADD"),
    (0x5c20, "IMNMX"),
    (0x5c28, "SHR"),
    (0x5c30, "FLO"),
    (0x5c40, "LOP"),
    (0x5c48, "SHL"),
    (0x5c58, "FADD"),
    (0x5c60, "FMNMX"),
    (0x5c68, "FMUL"),
    (0x5c70, "DADD"),
    (0x5c80, "DMUL"),
    (0x5c90, "RRO"),
    (0x5c98, "MOV"),
    (0x5ca0, "SEL"),
    (0x5ca8, "F2F"),
    (0x5cb0, "F2I"),
    (0x5cb8, "I2F"),
    (0x5cc0, "IADD3"),
    (0x5ce0, "I2I"),
    (0x5ce8, "P2R"),
    (0x5cf0, "R2P"),
    (0x5cf8, "SHF"),
];

/// Best-effort name for one instruction word; None when unrecognised.
pub fn identify(word: u64) -> Option<&'static str> {
    let top = (word >> 48) as u16;
    for &(value, name) in ALU_5C {
        if top & 0xfff8 == value || top & 0xfff8 == value - 0x1000 || top & 0xfef8 == value - 0x2400
        {
            return Some(name);
        }
    }
    TABLE
        .iter()
        .find(|&&(mask, value, _)| top & mask == value)
        .map(|&(_, _, name)| name)
}

/// The usual end-of-program self loop: BRA with offset -8 (PT).
pub const SELF_BRANCH: u64 = 0xe2400fffff87000f;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Census {
    /// Instructions scanned (scheduling words excluded).
    pub instructions: usize,
    pub known: BTreeMap<&'static str, usize>,
    /// Unrecognised words keyed by their top 16 bits.
    pub unknown: BTreeMap<u16, usize>,
    /// True when the trailing self branch ended the scan.
    pub terminated: bool,
}

/// Scan whole 32-byte bundles (scheduling word + 3 instructions) until the
/// end-of-program self branch; a trailing partial bundle is ignored.
pub fn scan(code: &[u8]) -> Census {
    let mut census = Census::default();
    for bundle in code.chunks_exact(32) {
        for slot in bundle[8..].chunks_exact(8) {
            let word = u64::from_le_bytes(slot.try_into().expect("word"));
            census.instructions += 1;
            match identify(word) {
                Some(name) => *census.known.entry(name).or_default() += 1,
                None => *census.unknown.entry((word >> 48) as u16).or_default() += 1,
            }
            if word == SELF_BRANCH {
                census.terminated = true;
            }
        }
        if census.terminated {
            break;
        }
    }
    census
}
