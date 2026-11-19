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

struct BitReader<'a> {
    bytes: &'a [u8; 16],
    pos: usize,
}
impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8; 16]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn read(&mut self, count: usize) -> u32 {
        let mut val = 0u32;
        for i in 0..count {
            let byte_idx = (self.pos + i) / 8;
            let bit_idx = (self.pos + i) % 8;
            let bit = (self.bytes[byte_idx] >> bit_idx) & 1;
            val |= (bit as u32) << i;
        }
        self.pos += count;
        val
    }
}

/// Decode one little-endian 16-byte BC7 (BPTC) block into 16 RGBA8 texels.
pub fn decode_bc7_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    let mut reader = BitReader::new(block);
    let mut mode = 0;
    while mode < 8 && reader.read(1) == 0 {
        mode += 1;
    }
    if mode == 6 {
        // Mode 6: 1 subset, 7-bit RGBA + 1 P-bit per endpoint, 4-bit index
        let r0 = reader.read(7);
        let r1 = reader.read(7);
        let g0 = reader.read(7);
        let g1 = reader.read(7);
        let b0 = reader.read(7);
        let b1 = reader.read(7);
        let a0 = reader.read(7);
        let a1 = reader.read(7);
        let p0 = reader.read(1);
        let p1 = reader.read(1);

        let ep0 = [
            ((r0 << 1) | p0) as u16,
            ((g0 << 1) | p0) as u16,
            ((b0 << 1) | p0) as u16,
            ((a0 << 1) | p0) as u16,
        ];
        let ep1 = [
            ((r1 << 1) | p1) as u16,
            ((g1 << 1) | p1) as u16,
            ((b1 << 1) | p1) as u16,
            ((a1 << 1) | p1) as u16,
        ];
        let weights = [
            0u16, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64,
        ];
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            let idx_bits = if i == 0 { 3 } else { 4 };
            let idx = reader.read(idx_bits) as usize;
            let w = weights[idx];
            for c in 0..4 {
                let interpolated = ((64 - w) * ep0[c] + w * ep1[c] + 32) >> 6;
                pixels[i][c] = interpolated.min(255) as u8;
            }
        }
        pixels
    } else if mode <= 5 {
        // Modes 0-5 RGB color fallback: reconstruct from endpoints
        let r0 = reader.read(6) << 2;
        let r1 = reader.read(6) << 2;
        let g0 = reader.read(6) << 2;
        let g1 = reader.read(6) << 2;
        let b0 = reader.read(6) << 2;
        let b1 = reader.read(6) << 2;
        let ep0 = [r0 as u16, g0 as u16, b0 as u16, 255u16];
        let ep1 = [r1 as u16, g1 as u16, b1 as u16, 255u16];
        let weights = [0u16, 21, 43, 64];
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            let idx = reader.read(2) as usize;
            let w = weights[idx & 3];
            for c in 0..3 {
                let interpolated = ((64 - w) * ep0[c] + w * ep1[c] + 32) >> 6;
                pixels[i][c] = interpolated.min(255) as u8;
            }
            pixels[i][3] = 255;
        }
        pixels
    } else {
        // Mode 7: 2 subsets, 5-bit RGBA
        let r0 = reader.read(5) << 3;
        let r1 = reader.read(5) << 3;
        let g0 = reader.read(5) << 3;
        let g1 = reader.read(5) << 3;
        let b0 = reader.read(5) << 3;
        let b1 = reader.read(5) << 3;
        let a0 = reader.read(5) << 3;
        let a1 = reader.read(5) << 3;
        let ep0 = [r0 as u16, g0 as u16, b0 as u16, a0 as u16];
        let ep1 = [r1 as u16, g1 as u16, b1 as u16, a1 as u16];
        let weights = [0u16, 21, 43, 64];
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            let idx = reader.read(2) as usize;
            let w = weights[idx & 3];
            for c in 0..4 {
                let interpolated = ((64 - w) * ep0[c] + w * ep1[c] + 32) >> 6;
                pixels[i][c] = interpolated.min(255) as u8;
            }
        }
        pixels
    }
}

/// Decode tightly packed BC7 (BPTC) 2D blocks, cropping partial edge blocks.
pub fn decode_bc7_rgba8(input: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DecodeError> {
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
        for (p, color) in decode_bc7_block(block.try_into().expect("block"))
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

/// NVIDIA Tegra TIC (Texture Image Control) descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TicHeader {
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub format: u32,
    pub block_linear_height_log2: u8,
}

impl TicHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() < 32 {
            return Err(DecodeError::InvalidLayout);
        }
        let w0 = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let w1 = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let w2 = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let w3 = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let w4 = u32::from_le_bytes(bytes[16..20].try_into().unwrap());

        let format = w0 & 0x7F;
        let address = (u64::from(w1 & 0xFF) << 32) | (u64::from(w1 & !0xFF));
        let width = (w4 & 0xFFFF) + 1;
        let height = ((w4 >> 16) & 0xFFFF) + 1;
        let depth = (w3 & 0x3FFF) + 1;
        let block_linear_height_log2 = ((w2 >> 3) & 7) as u8;

        Ok(Self {
            address,
            width,
            height,
            depth,
            format,
            block_linear_height_log2,
        })
    }
}

/// NVIDIA Tegra TSC (Texture Sampler Control) descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TscHeader {
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub wrap_r: u32,
    pub min_filter: u32,
    pub mag_filter: u32,
    pub mip_filter: u32,
}

impl TscHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() < 32 {
            return Err(DecodeError::InvalidLayout);
        }
        let w0 = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let w1 = u32::from_le_bytes(bytes[4..8].try_into().unwrap());

        let wrap_s = w0 & 7;
        let wrap_t = (w0 >> 3) & 7;
        let wrap_r = (w0 >> 6) & 7;
        let min_filter = (w1 >> 4) & 3;
        let mag_filter = (w1 >> 1) & 3;
        let mip_filter = (w1 >> 6) & 3;

        Ok(Self {
            wrap_s,
            wrap_t,
            wrap_r,
            min_filter,
            mag_filter,
            mip_filter,
        })
    }
}
