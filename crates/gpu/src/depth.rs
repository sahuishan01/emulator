//! Original bounded B197 ZF32 attachment state. Public NVIDIA clb197 fields.
use crate::{ColorClear, ExecutionError as E, GpuVa};

#[derive(Clone, Debug)]
pub struct DepthState {
    pub address: GpuVa,
    pub function: u32,
    pub write: bool,
    pub initial: Vec<u32>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Registers {
    target: [Option<u32>; 5],
    size: [Option<u32>; 3],
    read_only: Option<u32>,
    sparse: Option<u32>,
    select: Option<u32>,
    layer: Option<u32>,
    write: Option<u32>,
    function: Option<u32>,
    bounds: Option<u32>,
}
fn required(v: Option<u32>) -> Result<u32, E> {
    v.ok_or(E::UninitializedRegister)
}
impl Registers {
    pub(crate) fn write(&mut self, offset: u32, data: u32) -> bool {
        match offset {
            0xfe0..=0xff0 => self.target[((offset - 0xfe0) / 4) as usize] = Some(data),
            0x1228..=0x1230 => self.size[((offset - 0x1228) / 4) as usize] = Some(data),
            0x7f8 => self.read_only = Some(data),
            0x1208 => self.sparse = Some(data),
            0x1538 => self.select = Some(data),
            0x179c => self.layer = Some(data),
            0x12e8 => self.write = Some(data),
            0x130c => self.function = Some(data),
            0x19bc => self.bounds = Some(data),
            _ => return false,
        }
        true
    }
    pub(crate) fn plan(&self, color: ColorClear) -> Result<DepthState, E> {
        let mut target = [0; 5];
        for (a, b) in target.iter_mut().zip(self.target) {
            *a = required(b)?;
        }
        let mut size = [0; 3];
        for (a, b) in size.iter_mut().zip(self.size) {
            *a = required(b)?;
        }
        let [upper, lower, format, block, array_pitch] = target;
        let write = required(self.write)?;
        let function = required(self.function)?;
        let function = match function {
            0x200..=0x207 => function - 0x200,
            1..=8 => function - 1,
            _ => return Err(E::UnsupportedDrawState),
        };
        if upper > 255
            || lower & 511 != 0
            || format != 0xa
            || block != 0
            || array_pitch != 0
            || size != [color.width, color.height, 1]
            || color.width > 16
            || color.height > 8
            || write > 1
            || required(self.read_only)? != 1 - write
            || required(self.sparse)? != 0
            || required(self.select)? != 1
            || required(self.layer)? != 0
            || required(self.bounds)? != 0
        {
            return Err(E::UnsupportedDrawState);
        }
        Ok(DepthState {
            address: GpuVa((u64::from(upper) << 32) | u64::from(lower)),
            function,
            write: write != 0,
            initial: Vec::new(),
        })
    }
}
/// One generic 16Bx2 64-byte x 8-row GOB, bounded by attachment validation.
pub(crate) fn offset(x: usize, y: usize) -> usize {
    (x & 3) * 4 + (y & 1) * 16 + ((x >> 2) & 1) * 32 + ((y >> 1) & 3) * 64 + ((x >> 3) & 1) * 256
}
impl DepthState {
    pub(crate) fn passes(&self, incoming: f32, stored: f32) -> bool {
        match self.function {
            0 => false,
            1 => incoming < stored,
            2 => incoming == stored,
            3 => incoming <= stored,
            4 => incoming > stored,
            5 => incoming != stored,
            6 => incoming >= stored,
            7 => true,
            _ => false,
        }
    }
}
