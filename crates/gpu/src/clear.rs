//! Original MAXWELL_B color-clear subset, using NVIDIA's public clb197 interface.
//! One pitch-linear, single-layer UNORM8 render target; no shader execution.
use crate::{ExecutionError, GpuVa};

pub const MAXWELL_B: u32 = 0xb197;
pub const RGBA8_UNORM: u32 = 0xd5;
pub const BGRA8_UNORM: u32 = 0xcf;

/// Backend-neutral validated operation. Color remains in logical RGBA order;
/// format determines byte order in guest backing. Rect bounds are exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorClear {
    pub address: GpuVa,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: u32,
    pub rect: [u32; 4],
    pub color: [u32; 4],
    pub mask: u32,
}
impl ColorClear {
    pub fn rgba8(self) -> [u8; 4] {
        self.color
            .map(|v| (f32::from_bits(v).clamp(0.0, 1.0) * 255.0).round() as u8)
    }
    pub(crate) fn storage_color(self) -> ([u8; 4], u32) {
        let mut color = self.rgba8();
        let mut mask = self.mask;
        if self.format == BGRA8_UNORM {
            color.swap(0, 2);
            mask = (mask & 10) | ((mask & 1) << 2) | ((mask & 4) >> 2);
        }
        (color, mask)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ColorRegisters {
    target: [Option<u32>; 9],
    color: [Option<u32>; 4],
    clip: [Option<u32>; 2],
    clear_rect: [Option<u32>; 2],
    scissor: [Option<u32>; 3],
    control: Option<u32>,
}
fn set(value: Option<u32>) -> Result<u32, ExecutionError> {
    value.ok_or(ExecutionError::UninitializedRegister)
}
fn invalid() -> ExecutionError {
    ExecutionError::UnsupportedColorState
}
impl ColorRegisters {
    pub(crate) fn write(
        &mut self,
        offset: u32,
        data: u32,
    ) -> Result<Option<ColorClear>, ExecutionError> {
        match offset {
            0x100 => {}
            0x800..=0x820 => self.target[((offset - 0x800) / 4) as usize] = Some(data),
            0xd80..=0xd8c => self.color[((offset - 0xd80) / 4) as usize] = Some(data),
            0xd6c..=0xd70 => self.clear_rect[((offset - 0xd6c) / 4) as usize] = Some(data),
            0xe00..=0xe08 => self.scissor[((offset - 0xe00) / 4) as usize] = Some(data),
            0xff4..=0xff8 => self.clip[((offset - 0xff4) / 4) as usize] = Some(data),
            0x10f8 => self.control = Some(data),
            0x19d0 => return self.plan(data).map(Some),
            _ => {
                return Err(ExecutionError::UnsupportedMethod {
                    subchannel: 0,
                    offset,
                })
            }
        }
        Ok(None)
    }
    fn plan(&self, flags: u32) -> Result<ColorClear, ExecutionError> {
        // Reject depth/stencil, nonzero MRT/layer, empty masks and reserved bits.
        if flags == 0 || flags & !0x3c != 0 {
            return Err(invalid());
        }
        let mut target = [0; 9];
        for (out, input) in target.iter_mut().zip(self.target) {
            *out = set(input)?;
        }
        let [upper, lower, pitch, height, format, memory, third, array_pitch, layer] = target;
        if upper > 255
            || lower % 4 != 0
            || pitch == 0
            || pitch % 4 != 0
            || height == 0
            || height > 4096
            || pitch > 16384
            || ![RGBA8_UNORM, BGRA8_UNORM].contains(&format)
            || memory != 0x1000
            || third != 1
            || array_pitch != 0
            || layer != 0
        {
            return Err(invalid());
        }
        let clip = [set(self.clip[0])?, set(self.clip[1])?];
        let (width, clip_height) = (clip[0] >> 16, clip[1] >> 16);
        // Initial subset requires a zero-origin logical surface inside the pitch.
        if clip[0] & 65535 != 0
            || clip[1] & 65535 != 0
            || width == 0
            || width > 4096
            || width * 4 > pitch
            || clip_height != height
        {
            return Err(invalid());
        }
        let mut color = [0; 4];
        for (out, input) in color.iter_mut().zip(self.color) {
            *out = set(input)?;
            if !f32::from_bits(*out).is_finite() {
                return Err(invalid());
            }
        }
        let control = set(self.control)?;
        if control & !0x110 != 0 {
            return Err(invalid());
        }
        let mut rect = [0, 0, width, height];
        let mut intersect = |horizontal: u32, vertical: u32| -> Result<(), ExecutionError> {
            let r = [
                horizontal & 65535,
                vertical & 65535,
                horizontal >> 16,
                vertical >> 16,
            ];
            if r[0] > r[2] || r[1] > r[3] {
                return Err(invalid());
            }
            rect[0] = rect[0].max(r[0]);
            rect[1] = rect[1].max(r[1]);
            rect[2] = rect[2].min(r[2]);
            rect[3] = rect[3].min(r[3]);
            Ok(())
        };
        if control & 0x10 != 0 {
            intersect(set(self.clear_rect[0])?, set(self.clear_rect[1])?)?;
        }
        if control & 0x100 != 0 {
            match set(self.scissor[0])? {
                0 => {}
                1 => intersect(set(self.scissor[1])?, set(self.scissor[2])?)?,
                _ => return Err(invalid()),
            }
        }
        // Disjoint rectangles produce a valid empty clear, still one operation.
        rect[0] = rect[0].min(rect[2]);
        rect[1] = rect[1].min(rect[3]);
        Ok(ColorClear {
            address: GpuVa((u64::from(upper) << 32) | u64::from(lower)),
            width,
            height,
            pitch,
            format,
            rect,
            color,
            mask: flags >> 2,
        })
    }
}
