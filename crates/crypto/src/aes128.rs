//! AES-128 block cipher and bounded CTR/XTS operations.

use crate::CryptoError;

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];
const INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

/// AES with a 128-bit key. This lookup-table implementation is not constant-time.
#[derive(Clone)]
pub struct Aes128 {
    round_keys: [u8; 176],
}

impl Aes128 {
    pub fn new(key: &[u8; 16]) -> Self {
        let mut round_keys = [0; 176];
        round_keys[..16].copy_from_slice(key);
        let mut generated = 16;
        let mut rcon = 1u8;
        while generated < round_keys.len() {
            let mut temp = [
                round_keys[generated - 4],
                round_keys[generated - 3],
                round_keys[generated - 2],
                round_keys[generated - 1],
            ];
            if generated % 16 == 0 {
                temp.rotate_left(1);
                for byte in &mut temp {
                    *byte = SBOX[*byte as usize];
                }
                temp[0] ^= rcon;
                rcon = xtime(rcon);
            }
            for byte in temp {
                round_keys[generated] = round_keys[generated - 16] ^ byte;
                generated += 1;
            }
        }
        Self { round_keys }
    }

    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        add_round_key(block, &self.round_keys[..16]);
        for round in 1..=10 {
            for byte in block.iter_mut() {
                *byte = SBOX[*byte as usize];
            }
            shift_rows(block, false);
            if round != 10 {
                mix_columns(block, false);
            }
            add_round_key(block, &self.round_keys[round * 16..(round + 1) * 16]);
        }
    }

    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        add_round_key(block, &self.round_keys[160..176]);
        for round in (1..10).rev() {
            shift_rows(block, true);
            for byte in block.iter_mut() {
                *byte = INV_SBOX[*byte as usize];
            }
            add_round_key(block, &self.round_keys[round * 16..(round + 1) * 16]);
            mix_columns(block, true);
        }
        shift_rows(block, true);
        for byte in block.iter_mut() {
            *byte = INV_SBOX[*byte as usize];
        }
        add_round_key(block, &self.round_keys[..16]);
    }
}

/// XOR input with AES-CTR keystream using a 128-bit big-endian counter.
/// Counter layout must be adapted by the caller for formats with other rules.
pub fn aes128_ctr_xor(
    key: &[u8; 16],
    initial_counter: [u8; 16],
    data: &mut [u8],
) -> Result<(), CryptoError> {
    let blocks = data.len().div_ceil(16);
    if blocks > 1 {
        let increments = (blocks - 1) as u128;
        let initial = u128::from_be_bytes(initial_counter);
        if initial.checked_add(increments).is_none() {
            return Err(CryptoError::CounterWouldWrap);
        }
    }

    let aes = Aes128::new(key);
    let mut counter = initial_counter;
    for chunk in data.chunks_mut(16) {
        let mut stream = counter;
        aes.encrypt_block(&mut stream);
        for (byte, mask) in chunk.iter_mut().zip(stream) {
            *byte ^= mask;
        }
        increment_be(&mut counter);
    }
    Ok(())
}

/// Decrypt one or more 16-byte blocks using AES-128 in ECB mode.
pub fn aes128_ecb_decrypt(
    key: &[u8; 16],
    data: &mut [u8],
) -> Result<(), CryptoError> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err(CryptoError::EcbInputMustContainWholeBlocks);
    }
    let aes = Aes128::new(key);
    for chunk in data.chunks_exact_mut(16) {
        let block: &mut [u8; 16] = chunk.try_into().expect("16-byte block");
        aes.decrypt_block(block);
    }
    Ok(())
}

/// Transform one XTS data unit using two AES-128 keys and a caller-provided
/// 128-bit tweak input. Data units must be at least one full block and a
/// multiple of 16 bytes; ciphertext stealing is intentionally unsupported.
pub fn aes128_xts_decrypt(
    key: &[u8; 32],
    tweak_input: [u8; 16],
    data: &mut [u8],
) -> Result<(), CryptoError> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err(CryptoError::XtsInputMustContainWholeBlocks);
    }
    let data_key = Aes128::new(key[..16].try_into().expect("fixed key slice"));
    let tweak_key = Aes128::new(key[16..].try_into().expect("fixed key slice"));
    let mut tweak = tweak_input;
    tweak_key.encrypt_block(&mut tweak);
    for chunk in data.chunks_exact_mut(16) {
        let mut block: [u8; 16] = chunk.try_into().expect("fixed block");
        xor_block(&mut block, &tweak);
        data_key.decrypt_block(&mut block);
        xor_block(&mut block, &tweak);
        chunk.copy_from_slice(&block);
        multiply_alpha(&mut tweak);
    }
    Ok(())
}

/// Transform one complete XTS data unit in the forward direction.
pub fn aes128_xts_encrypt(
    key: &[u8; 32],
    tweak_input: [u8; 16],
    data: &mut [u8],
) -> Result<(), CryptoError> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err(CryptoError::XtsInputMustContainWholeBlocks);
    }
    let data_key = Aes128::new(key[..16].try_into().expect("fixed key slice"));
    let tweak_key = Aes128::new(key[16..].try_into().expect("fixed key slice"));
    let mut tweak = tweak_input;
    tweak_key.encrypt_block(&mut tweak);
    for chunk in data.chunks_exact_mut(16) {
        let mut block: [u8; 16] = chunk.try_into().expect("fixed block");
        xor_block(&mut block, &tweak);
        data_key.encrypt_block(&mut block);
        xor_block(&mut block, &tweak);
        chunk.copy_from_slice(&block);
        multiply_alpha(&mut tweak);
    }
    Ok(())
}

fn add_round_key(state: &mut [u8; 16], key: &[u8]) {
    for (byte, round_key) in state.iter_mut().zip(key) {
        *byte ^= round_key;
    }
}

fn shift_rows(state: &mut [u8; 16], inverse: bool) {
    let source = *state;
    for row in 0..4 {
        for column in 0..4 {
            let from = if inverse {
                (column + 4 - row) % 4
            } else {
                (column + row) % 4
            };
            state[row + 4 * column] = source[row + 4 * from];
        }
    }
}

fn mix_columns(state: &mut [u8; 16], inverse: bool) {
    for column in 0..4 {
        let i = column * 4;
        let [a, b, c, d] = [state[i], state[i + 1], state[i + 2], state[i + 3]];
        if inverse {
            state[i] = mul(a, 14) ^ mul(b, 11) ^ mul(c, 13) ^ mul(d, 9);
            state[i + 1] = mul(a, 9) ^ mul(b, 14) ^ mul(c, 11) ^ mul(d, 13);
            state[i + 2] = mul(a, 13) ^ mul(b, 9) ^ mul(c, 14) ^ mul(d, 11);
            state[i + 3] = mul(a, 11) ^ mul(b, 13) ^ mul(c, 9) ^ mul(d, 14);
        } else {
            state[i] = xtime(a) ^ (xtime(b) ^ b) ^ c ^ d;
            state[i + 1] = a ^ xtime(b) ^ (xtime(c) ^ c) ^ d;
            state[i + 2] = a ^ b ^ xtime(c) ^ (xtime(d) ^ d);
            state[i + 3] = (xtime(a) ^ a) ^ b ^ c ^ xtime(d);
        }
    }
}

fn mul(mut value: u8, mut factor: u8) -> u8 {
    let mut result = 0;
    while factor != 0 {
        if factor & 1 != 0 {
            result ^= value;
        }
        value = xtime(value);
        factor >>= 1;
    }
    result
}

fn xtime(value: u8) -> u8 {
    (value << 1) ^ if value & 0x80 != 0 { 0x1b } else { 0 }
}

fn increment_be(counter: &mut [u8; 16]) {
    for byte in counter.iter_mut().rev() {
        let (next, overflow) = byte.overflowing_add(1);
        *byte = next;
        if !overflow {
            break;
        }
    }
}

fn xor_block(left: &mut [u8; 16], right: &[u8; 16]) {
    for (left, right) in left.iter_mut().zip(right) {
        *left ^= right;
    }
}

fn multiply_alpha(tweak: &mut [u8; 16]) {
    let mut carry = 0;
    for byte in tweak.iter_mut() {
        let next = *byte >> 7;
        *byte = (*byte << 1) | carry;
        carry = next;
    }
    if carry != 0 {
        tweak[0] ^= 0x87;
    }
}
