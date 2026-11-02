//! Original SPH reader from NVIDIA's public 80-byte header specification.
//! One exact v3/SASS1 vertex or pixel interface; all other fields fail closed.
use crate::{decode, Error, Program, Stage};
pub const HEADER_BYTES: usize = 80;
pub fn decode_program(bytes: &[u8], stage: Stage) -> Result<Program, Error> {
    if bytes.len() < HEADER_BYTES {
        return Err(Error::Header);
    }
    let mut expected = [0u32; 20];
    expected[0] = match stage {
        Stage::Vertex => 1 | (3 << 5) | (1 << 10) | (1 << 17) | (1 << 26),
        Stage::Fragment => 2 | (3 << 5) | (5 << 10) | (1 << 17),
    };
    if u32::from_le_bytes(bytes[..4].try_into().expect("word")) != expected[0] {
        return Err(Error::Header);
    }
    let program = decode(&bytes[HEADER_BYTES..], stage)?;
    expected[4] = 1 << 12; // StoreReqStart 1 > StoreReqEnd 0: no forced exports.
    match stage {
        Stage::Vertex => {
            expected[6] = 15;
            if program.input_mask() & 0xf0 != 0 {
                expected[6] |= 0xf0;
            }
            expected[13] = 0xf000;
            if program.varying_outputs().is_some() {
                expected[14] = 15;
            }
        }
        Stage::Fragment => {
            expected[18] = 15;
            for c in 0..4 {
                if program.input_mask() & (1 << c) != 0 {
                    expected[6] |= 3 << (c * 2);
                }
            }
        }
    }
    for (word, want) in bytes[..HEADER_BYTES].chunks_exact(4).zip(expected) {
        if u32::from_le_bytes(word.try_into().expect("word")) != want {
            return Err(Error::Header);
        }
    }
    Ok(program)
}
