//! Original narrow Maxwell pitch-linear texture/sampler descriptor reader.
use crate::{ExecutionError as E, GpuAddressSpace, GpuMemoryRange, GpuVa};
use emulator_memory::Access;

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
    let rgba8 = 8
        | (2 << 7)
        | (2 << 10)
        | (2 << 13)
        | (2 << 16)
        | (2 << 19)
        | (3 << 22)
        | (4 << 25)
        | (5 << 28);
    let width = (header[4] & 65535) + 1;
    let height = (header[5] & 65535) + 1;
    let pitch = (header[3] & 65535) * 32;
    if header[0] != rgba8
        || header[1] & 31 != 0
        || header[2] & !255 != 2 << 21
        || header[3] >> 16 != 0
        || header[4] & !65535 != 1 << 23
        || header[5] & !65535 != 1 << 31
        || header[6] != 0
        || header[7] != 0
        || width > 64
        || height > 64
        || pitch < width * 4
        || pitch > 256
        || sampler != [0x92, 0x51, 0, 0, 0, 0, 0, 0]
    {
        return Err(invalid());
    }
    let address = (u64::from(header[2] & 255) << 32) | u64::from(header[1]);
    let pin = space
        .pin(GpuVa(address), (pitch * height) as usize, Access::Read)
        .map_err(E::Memory)?;
    let mut bytes = vec![0; (pitch * height) as usize];
    pin.read(&mut bytes).map_err(E::Memory)?;
    pins.push(pin);
    let pixels = bytes
        .chunks_exact(pitch as usize)
        .flat_map(|row| row[..width as usize * 4].iter().copied())
        .collect();
    Ok(SampledTexture {
        width,
        height,
        pixels,
    })
}
