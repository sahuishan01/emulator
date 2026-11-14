//! Clean-room LZ4 block decompression implementation.
//!
//! Nintendo Switch NSO binaries use standard raw LZ4 block compression for
//! .text, .rodata, and .data sections.

use crate::LoadError;

/// Decompresses an uncompressed or LZ4 block-compressed payload.
///
/// `compressed` is the raw LZ4 block stream.
/// `decompressed_size` is the expected output byte count.
pub fn lz4_decompress(compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>, LoadError> {
    let mut output = Vec::with_capacity(decompressed_size);
    let mut cursor = 0;

    while cursor < compressed.len() {
        let token = compressed[cursor];
        cursor += 1;

        // 1. Literal length from high nibble
        let mut literal_len = (token >> 4) as usize;
        if literal_len == 15 {
            loop {
                if cursor >= compressed.len() {
                    return Err(LoadError::Invalid("LZ4 stream truncated in literal length"));
                }
                let extra = compressed[cursor];
                cursor += 1;
                literal_len = literal_len
                    .checked_add(extra as usize)
                    .ok_or(LoadError::LimitExceeded)?;
                if extra < 255 {
                    break;
                }
            }
        }

        // Copy literals
        if cursor + literal_len > compressed.len() {
            return Err(LoadError::Invalid("LZ4 literal length exceeds input bounds"));
        }
        if output.len() + literal_len > decompressed_size {
            return Err(LoadError::Invalid("LZ4 output exceeds expected decompressed size"));
        }
        output.extend_from_slice(&compressed[cursor..cursor + literal_len]);
        cursor += literal_len;

        // If at the end of the input, the sequence is complete (last sequence has no match)
        if cursor >= compressed.len() {
            break;
        }

        // 2. Match offset (2 bytes, little-endian)
        if cursor + 2 > compressed.len() {
            return Err(LoadError::Invalid("LZ4 stream truncated in match offset"));
        }
        let offset = u16::from_le_bytes([compressed[cursor], compressed[cursor + 1]]) as usize;
        cursor += 2;

        if offset == 0 || offset > output.len() {
            return Err(LoadError::Invalid("LZ4 match offset is invalid or points out of bounds"));
        }

        // 3. Match length from low nibble (plus minimum match 4)
        let mut match_len = (token & 0x0F) as usize;
        if match_len == 15 {
            loop {
                if cursor >= compressed.len() {
                    return Err(LoadError::Invalid("LZ4 stream truncated in match length"));
                }
                let extra = compressed[cursor];
                cursor += 1;
                match_len = match_len
                    .checked_add(extra as usize)
                    .ok_or(LoadError::LimitExceeded)?;
                if extra < 255 {
                    break;
                }
            }
        }
        match_len += 4;

        if output.len() + match_len > decompressed_size {
            return Err(LoadError::Invalid("LZ4 match output exceeds expected decompressed size"));
        }

        // Copy matching bytes (byte-by-byte to handle overlapping repeats)
        let match_start = output.len() - offset;
        for i in 0..match_len {
            let byte = output[match_start + i];
            output.push(byte);
        }
    }

    if output.len() != decompressed_size {
        return Err(LoadError::Invalid("LZ4 decompressed size did not match expected size"));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lz4_literals_only() {
        // Token 0x50: 5 literals, 0 match length (end of block)
        let compressed = [0x50, b'H', b'e', b'l', b'l', b'o'];
        let decompressed = lz4_decompress(&compressed, 5).expect("decompress should succeed");
        assert_eq!(&decompressed, b"Hello");
    }

    #[test]
    fn test_lz4_match_and_repetition() {
        // "ABCDEF" followed by match offset 6, length 4 (0 + 4 = 4) -> "ABCD" -> total "ABCDEFABCD"
        // Token: 6 literals (0x60), match length 0 (0x00) -> 0x60
        // Literals: 'A', 'B', 'C', 'D', 'E', 'F'
        // Offset: 6, 0 (0x0006)
        // Then end of sequence: 0 literals, end: token 0x00
        let compressed = [
            0x60, b'A', b'B', b'C', b'D', b'E', b'F',
            0x06, 0x00,
        ];
        let decompressed = lz4_decompress(&compressed, 10).expect("decompress should succeed");
        assert_eq!(&decompressed, b"ABCDEFABCD");
    }

    #[test]
    fn test_lz4_run_length_match() {
        // 1 literal 'X', offset 1, match length 7 (3 + 4) -> "XXXXXXXX" (8 bytes total)
        // Token: literal 1, match 3 -> 0x13
        // Literal: 'X'
        // Offset: 1, 0 (0x0001)
        let compressed = [0x13, b'X', 0x01, 0x00];
        let decompressed = lz4_decompress(&compressed, 8).expect("run-length match should succeed");
        assert_eq!(&decompressed, b"XXXXXXXX");
    }

    #[test]
    fn test_lz4_invalid_offset() {
        // Offset 0 is invalid in LZ4
        let compressed = [0x10, b'A', 0x00, 0x00];
        assert!(lz4_decompress(&compressed, 5).is_err());
    }
}
