//! Original bounded BC1/BC3 UNORM decoding and 2D pitch/block-linear storage.
//! No sRGB conversion, mip chains, arrays or compressed render targets.

mod storage;
pub use storage::{storage_size, unpack_2d, StorageLayout};

use std::fmt;

pub const MAX_DECODED_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    InvalidDimensions,
    SizeOverflow,
    AllocationLimit,
    InputLength { expected: usize, actual: usize },
    AllocationFailed,
    InvalidLayout,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => write!(f, "texture dimensions must be nonzero"),
            Self::SizeOverflow => write!(f, "texture size overflows"),
            Self::AllocationLimit => write!(f, "decoded texture exceeds the allocation limit"),
            Self::InputLength { expected, actual } => {
                write!(f, "texture input has {actual} bytes, expected {expected}")
            }
            Self::AllocationFailed => write!(f, "decoded texture allocation failed"),
            Self::InvalidLayout => write!(f, "unsupported or invalid texture storage layout"),
        }
    }
}
impl std::error::Error for DecodeError {}

/// Expand one little-endian BC1 RGBA block into row-major RGBA8 pixels.
/// Endpoints are normalized from RGB565, interpolation uses that precision,
/// and the result is rounded to the nearest UNORM8 value once, with ties up.
pub fn decode_bc1_block(block: &[u8; 8]) -> [[u8; 4]; 16] {
    colors(block, false)
}
fn colors(block: &[u8; 8], four: bool) -> [[u8; 4]; 16] {
    let first = u16::from_le_bytes([block[0], block[1]]);
    let second = u16::from_le_bytes([block[2], block[3]]);
    let indices = u32::from_le_bytes(block[4..8].try_into().expect("fixed block"));
    let mut palette = [[0_u8; 4]; 4];
    for (index, color) in palette.iter_mut().enumerate() {
        if !four && first <= second && index == 3 {
            continue; // Transparent black in the RGBA three-color mode.
        }
        color[3] = 255;
        for (channel, (shift, mask)) in [(11, 31_u32), (5, 63), (0, 31)].into_iter().enumerate() {
            let a = (u32::from(first) >> shift) & mask;
            let b = (u32::from(second) >> shift) & mask;
            let (value, divisor) = match index {
                0 => (a, mask),
                1 => (b, mask),
                2 if four || first > second => (2 * a + b, 3 * mask),
                3 => (a + 2 * b, 3 * mask),
                _ => (a + b, 2 * mask),
            };
            color[channel] = ((value * 255 + divisor / 2) / divisor) as u8;
        }
    }
    std::array::from_fn(|pixel| palette[((indices >> (2 * pixel)) & 3) as usize])
}

/// DXT5/BC3: a little-endian 48-bit alpha index stream and an always-four-color
/// RGB block. Interpolation is normalized first, rounded to UNORM8 once (ties up).
pub fn decode_bc3_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    let a = u32::from(block[0]);
    let b = u32::from(block[1]);
    let mut alpha = [0u8; 8];
    alpha[0] = a as u8;
    alpha[1] = b as u8;
    for (i, out) in alpha.iter_mut().enumerate().skip(2) {
        *out = if a > b {
            (((8 - i) as u32 * a + (i - 1) as u32 * b + 3) / 7) as u8
        } else if i < 6 {
            (((6 - i) as u32 * a + (i - 1) as u32 * b + 2) / 5) as u8
        } else if i == 6 {
            0
        } else {
            255
        };
    }
    let mut indices = [0u8; 8];
    indices[..6].copy_from_slice(&block[2..8]);
    let indices = u64::from_le_bytes(indices);
    let mut out = colors(block[8..].try_into().expect("color block"), true);
    for (i, p) in out.iter_mut().enumerate() {
        p[3] = alpha[((indices >> (3 * i)) & 7) as usize];
    }
    out
}

/// Decode tightly packed BC3 2D blocks, cropping partial edge blocks.
pub fn decode_bc3_rgba8(input: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DecodeError> {
    if width == 0 || height == 0 {
        return Err(DecodeError::InvalidDimensions);
    }
    let width = usize::try_from(width).map_err(|_| DecodeError::SizeOverflow)?;
    let height = usize::try_from(height).map_err(|_| DecodeError::SizeOverflow)?;
    let size = width
        .checked_mul(height)
        .and_then(|v| v.checked_mul(4))
        .ok_or(DecodeError::SizeOverflow)?;
    if size > MAX_DECODED_BYTES {
        return Err(DecodeError::AllocationLimit);
    }
    let bx = width.div_ceil(4);
    let expected = bx
        .checked_mul(height.div_ceil(4))
        .and_then(|v| v.checked_mul(16))
        .ok_or(DecodeError::SizeOverflow)?;
    if input.len() != expected {
        return Err(DecodeError::InputLength {
            expected,
            actual: input.len(),
        });
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(size)
        .map_err(|_| DecodeError::AllocationFailed)?;
    output.resize(size, 0);
    for (i, block) in input.chunks_exact(16).enumerate() {
        for (p, color) in decode_bc3_block(block.try_into().expect("block"))
            .iter()
            .enumerate()
        {
            let x = i % bx * 4 + p % 4;
            let y = i / bx * 4 + p / 4;
            if x < width && y < height {
                let a = (y * width + x) * 4;
                output[a..a + 4].copy_from_slice(color);
            }
        }
    }
    Ok(output)
}

/// Decode a tightly packed 2D image. Edge blocks are cropped to its dimensions.
pub fn decode_bc1_rgba8(input: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DecodeError> {
    if width == 0 || height == 0 {
        return Err(DecodeError::InvalidDimensions);
    }
    let width = usize::try_from(width).map_err(|_| DecodeError::SizeOverflow)?;
    let height = usize::try_from(height).map_err(|_| DecodeError::SizeOverflow)?;
    let output_size = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or(DecodeError::SizeOverflow)?;
    if output_size > MAX_DECODED_BYTES {
        return Err(DecodeError::AllocationLimit);
    }
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let expected = blocks_x
        .checked_mul(blocks_y)
        .and_then(|n| n.checked_mul(8))
        .ok_or(DecodeError::SizeOverflow)?;
    if input.len() != expected {
        return Err(DecodeError::InputLength {
            expected,
            actual: input.len(),
        });
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_size)
        .map_err(|_| DecodeError::AllocationFailed)?;
    output.resize(output_size, 0);
    for (index, block) in input.chunks_exact(8).enumerate() {
        let pixels = decode_bc1_block(block.try_into().expect("complete block"));
        for (pixel, color) in pixels.iter().enumerate() {
            let x = (index % blocks_x) * 4 + pixel % 4;
            let y = (index / blocks_x) * 4 + pixel / 4;
            if x < width && y < height {
                let offset = (y * width + x) * 4;
                output[offset..offset + 4].copy_from_slice(color);
            }
        }
    }
    Ok(output)
}
