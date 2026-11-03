//! Synchronous host graphics contract for the explicitly bounded live slice.
use crate::{ColorClear, Draw, PreparedSubmission};

#[derive(Clone, Debug)]
pub struct GraphicsFrame {
    pub background: ColorClear,
    pub draws: Vec<Draw>,
    /// Independent serial shader/raster coverage oracle, before publication.
    pub pixels: Vec<u8>,
}
/// Called only after the kernel validates all effects, replies and handles.
/// Return only after actual host completion and exact oracle verification.
/// Must not reenter the guest/kernel or retain borrowed state. Failure prevents
/// guest bytes, channel state, completion values and replies from publishing.
pub trait GraphicsBackend: std::fmt::Debug {
    fn render(&self, frame: &GraphicsFrame) -> Result<(), String>;
}
impl PreparedSubmission {
    pub fn graphics_frame(&self) -> Result<GraphicsFrame, String> {
        let clears = self.color_clears();
        let draws = self.draws();
        if self.stats().copy_launches != 0
            || clears.len() != 1
            || draws.is_empty()
            || draws.len() > 3
            || self.batch.graphics_order.first() != Some(&false)
            || self.batch.graphics_order[1..].iter().any(|draw| !draw)
        {
            return Err("Live backend requires one full clear then 1..3 draws, without DMA".into());
        }
        let c = clears[0];
        if c.width == 0
            || c.height == 0
            || c.width > 64
            || c.height > 64
            || c.pitch != c.width * 4
            || c.mask != 15
            || c.rect != [0, 0, c.width, c.height]
        {
            return Err("Unsupported live framebuffer layout".into());
        }
        let (color, _) = c.storage_color();
        let mut pixels = color.repeat((c.width * c.height) as usize);
        for draw in draws {
            let s = draw.surface;
            if s.address != c.address
                || s.width != c.width
                || s.height != c.height
                || s.pitch != c.pitch
                || s.format != c.format
                || s.mask != 15
                || draw.vertices.len() > 6
            {
                return Err("Live draws must share the framebuffer and have <=6 vertices".into());
            }
            let raster = draw.raster().map_err(|e| e.to_string())?;
            for (pixel, color) in pixels.chunks_exact_mut(4).zip(raster) {
                if let Some(color) = color {
                    pixel.copy_from_slice(&color);
                }
            }
        }
        // The independently composed shader/raster oracle must agree with the
        // memory effects that will actually publish. Reuse the retained first
        // clear's target pins; no GPU address lookup or new lifetime is needed.
        self.batch.verify_frame_pixels(&pixels)?;
        Ok(GraphicsFrame {
            background: c,
            draws: draws.to_vec(),
            pixels,
        })
    }
}
