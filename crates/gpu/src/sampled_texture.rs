//! Original bounded Maxwell texture descriptors, layouts and RGBA fallback.
use crate::{ExecutionError as E, GpuAddressSpace, GpuMemoryRange, GpuVa};
use emulator_memory::Access;
use emulator_texture::{
    decode_bc1_rgba8, decode_bc3_rgba8, storage_size, unpack_2d, StorageLayout,
};

#[derive(Clone, Debug)]
pub struct SampledTexture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
impl SampledTexture {
    pub fn image(&self) -> emulator_shader::SampledImage<'_> {
        emulator_shader::SampledImage {
            width: self.width,
            height: self.height,
            pixels: &self.pixels,
        }
    }
}
pub(crate) fn capture(
    space: &GpuAddressSpace,
    pools: [[u32; 3]; 2],
    handle: u32,
    pins: &mut Vec<GpuMemoryRange>,
) -> Result<SampledTexture, E> {
    let invalid = || E::UnsupportedDrawState;
    let mut descriptors = [[0u32; 8]; 2];
    for (i, index) in [handle & 0xfffff, handle >> 20].into_iter().enumerate() {
        let [upper, lower, limit] = pools[i];
        if upper > 255
            || lower & 255 != 0
            || limit > if i == 0 { 0x3fffff } else { 0xfffff }
            || index > limit
        {
            return Err(invalid());
        }
        let address = ((u64::from(upper) << 32) | u64::from(lower))
            .checked_add(u64::from(index) * 32)
            .ok_or_else(invalid)?;
        let pin = space
            .pin(GpuVa(address), 32, Access::Read)
            .map_err(E::Memory)?;
        let mut bytes = [0; 32];
        pin.read(&mut bytes).map_err(E::Memory)?;
        pins.push(pin);
        for (word, bytes) in descriptors[i].iter_mut().zip(bytes.chunks_exact(4)) {
            *word = u32::from_le_bytes(bytes.try_into().expect("word"));
        }
    }
    let [header, sampler] = descriptors;
    let components = (2 << 7)
        | (2 << 10)
        | (2 << 13)
        | (2 << 16)
        | (2 << 19)
        | (3 << 22)
        | (4 << 25)
        | (5 << 28);
    let width = (header[4] & 65535) + 1;
    let height = (header[5] & 65535) + 1;
    let format = header[0] & 63;
    let version = header[2] >> 21;
    let layout = match version {
        2 if header[3] >> 16 == 0 => {
            let pitch = (header[3] & 65535) as usize * 32;
            if pitch > 256 {
                return Err(invalid());
            }
            StorageLayout::Pitch { pitch }
        }
        3 if header[3] & !0x38 == 0 => StorageLayout::BlockLinear {
            height_log2: ((header[3] >> 3) & 7) as u8,
        },
        _ => return Err(invalid()),
    };
    if header[0] != components | format
        || ![8, 0x24, 0x26].contains(&format)
        || header[1] & if version == 3 { 511 } else { 31 } != 0
        || header[2] & !255 != version << 21
        || header[4] & !65535 != 1 << 23
        || header[5] & !65535 != 1 << 31
        || header[6] != 0
        || header[7] != 0
        || width > 64
        || height > 64
        || sampler != [0x92, 0x51, 0, 0, 0, 0, 0, 0]
    {
        return Err(invalid());
    }
    let (row_bytes, rows) = if format == 8 {
        (width as usize * 4, height as usize)
    } else {
        (
            width.div_ceil(4) as usize * if format == 0x24 { 8 } else { 16 },
            height.div_ceil(4) as usize,
        )
    };
    let size = storage_size(row_bytes, rows, layout).map_err(|_| invalid())?;
    let address = (u64::from(header[2] & 255) << 32) | u64::from(header[1]);
    let pin = space
        .pin(GpuVa(address), size, Access::Read)
        .map_err(E::Memory)?;
    let mut bytes = vec![0; size];
    pin.read(&mut bytes).map_err(E::Memory)?;
    pins.push(pin);
    let packed = unpack_2d(&bytes, row_bytes, rows, layout).map_err(|_| invalid())?;
    let pixels = match format {
        8 => packed,
        0x24 => decode_bc1_rgba8(&packed, width, height).map_err(|_| invalid())?,
        0x26 => decode_bc3_rgba8(&packed, width, height).map_err(|_| invalid())?,
        _ => return Err(invalid()),
    };
    Ok(SampledTexture {
        width,
        height,
        pixels,
    })
}
