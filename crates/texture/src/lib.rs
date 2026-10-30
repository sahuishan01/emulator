//! Original reference decoding of tightly packed, linear BC1 RGBA UNORM blocks.
//! No sRGB conversion, Tegra block-linear addressing, mip chains or other BC formats.

use std::fmt;

pub const MAX_DECODED_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    InvalidDimensions,
    SizeOverflow,
    AllocationLimit,
    InputLength { expected: usize, actual: usize },
    AllocationFailed,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => write!(f, "texture dimensions must be nonzero"),
            Self::SizeOverflow => write!(f, "texture size overflows"),
            Self::AllocationLimit => write!(f, "decoded texture exceeds the allocation limit"),
            Self::InputLength { expected, actual } => {
                write!(f, "BC1 input has {actual} bytes, expected {expected}")
            }
            Self::AllocationFailed => write!(f, "decoded texture allocation failed"),
        }
    }
}
impl std::error::Error for DecodeError {}

/// Expand one little-endian BC1 RGBA block into row-major RGBA8 pixels.
/// Endpoints are normalized from RGB565, interpolation uses that precision,
/// and the result is rounded to the nearest UNORM8 value once, with ties up.
pub fn decode_bc1_block(block: &[u8; 8]) -> [[u8; 4]; 16] {
    let first = u16::from_le_bytes([block[0], block[1]]);
    let second = u16::from_le_bytes([block[2], block[3]]);
    let indices = u32::from_le_bytes(block[4..8].try_into().expect("fixed block"));
    let mut palette = [[0_u8; 4]; 4];
    for (index, color) in palette.iter_mut().enumerate() {
        if first <= second && index == 3 {
            continue; // Transparent black in the RGBA three-color mode.
        }
        color[3] = 255;
        for (channel, (shift, mask)) in [(11, 31_u32), (5, 63), (0, 31)].into_iter().enumerate() {
            let a = (u32::from(first) >> shift) & mask;
            let b = (u32::from(second) >> shift) & mask;
            let (value, divisor) = match index {
                0 => (a, mask),
                1 => (b, mask),
                2 if first > second => (2 * a + b, 3 * mask),
                3 => (a + 2 * b, 3 * mask),
                _ => (a + b, 2 * mask),
            };
            color[channel] = ((value * 255 + divisor / 2) / divisor) as u8;
        }
    }
    std::array::from_fn(|pixel| palette[((indices >> (2 * pixel)) & 3) as usize])
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
