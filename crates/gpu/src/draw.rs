//! Original bounded B197 non-indexed triangle and shader input preparation.
use crate::{
    clear::ColorRegisters, ColorClear, ExecutionError as E, GpuAddressSpace, GpuMemoryRange, GpuVa,
};
use emulator_memory::Access;
use emulator_shader::{decode_program, emit, Program, Stage};

/// Owned host-independent draw. Captured inputs precede guest shader execution;
/// surface color is used only for the uniform fragment fast path.
#[derive(Clone, Debug)]
pub struct Draw {
    pub surface: ColorClear,
    pub vertices: Vec<[u32; 4]>,
    pub varying_inputs: Vec<[u32; 4]>,
    pub texture: Option<crate::SampledTexture>,
    pub vertex: Program,
    pub fragment: Program,
    pub vertex_spirv: Vec<u32>,
    pub fragment_spirv: Vec<u32>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Registers {
    viewport: [Option<u32>; 6],
    clip: [Option<u32>; 4],
    stream: [Option<u32>; 4],
    limit: [Option<u32>; 2],
    attribute: Option<u32>,
    attribute1: Option<u32>,
    texture_mode: Option<u32>,
    texture_pools: [[Option<u32>; 3]; 2],
    region: [Option<u32>; 2],
    pipeline: [[Option<u32>; 3]; 6],
    disabled: [Option<u32>; 4],
    origin: Option<u32>,
    scale: Option<u32>,
    mask: Option<u32>,
    targets: Option<u32>,
}
fn required(v: Option<u32>) -> Result<u32, E> {
    v.ok_or(E::UninitializedRegister)
}
fn invalid() -> E {
    E::UnsupportedDrawState
}
impl Registers {
    /// None means method belongs to the color engine. A returned draw command
    /// encodes topology/count/first in the public BEGIN_END_INSTANCE_FIRST word.
    pub(crate) fn write(&mut self, offset: u32, data: u32) -> Result<Option<Option<u32>>, E> {
        match offset {
            0xa00..=0xa14 => self.viewport[((offset - 0xa00) / 4) as usize] = Some(data),
            0xc00..=0xc0c => self.clip[((offset - 0xc00) / 4) as usize] = Some(data),
            0x1c00..=0x1c0c => self.stream[((offset - 0x1c00) / 4) as usize] = Some(data),
            0x1f00..=0x1f04 => self.limit[((offset - 0x1f00) / 4) as usize] = Some(data),
            0x1160 => self.attribute = Some(data),
            0x1164 => self.attribute1 = Some(data),
            0xf10 => self.texture_mode = Some(data),
            0x1574..=0x157c => self.texture_pools[0][((offset - 0x1574) / 4) as usize] = Some(data),
            0x155c..=0x1564 => self.texture_pools[1][((offset - 0x155c) / 4) as usize] = Some(data),
            0x1608..=0x160c => self.region[((offset - 0x1608) / 4) as usize] = Some(data),
            0x2000..=0x214c => {
                let slot = ((offset - 0x2000) / 64) as usize;
                let field = match (offset - 0x2000) % 64 {
                    0 => 0,
                    4 => 1,
                    12 => 2,
                    _ => return Ok(None),
                };
                self.pipeline[slot][field] = Some(data);
            }
            0x12cc => self.disabled[0] = Some(data), // depth test
            0x1360 => self.disabled[1] = Some(data), // blend target 0
            0x1918 => self.disabled[2] = Some(data), // OGL cull
            0x1380 => self.disabled[3] = Some(data), // stencil test
            0x13ac => self.origin = Some(data),
            0x192c => self.scale = Some(data),
            0x1a00 => self.mask = Some(data),
            0x121c => self.targets = Some(data),
            0x1214 => return Ok(Some(Some(data))),
            _ => return Ok(None),
        }
        Ok(Some(None))
    }
    pub(crate) fn plan(
        &self,
        space: &GpuAddressSpace,
        color: &ColorRegisters,
        command: u32,
    ) -> Result<(Draw, Vec<GpuMemoryRange>), E> {
        let count = ((command >> 16) & 0xfff) as usize;
        let first = (command & 65535) as usize;
        if command >> 28 != 4 || count == 0 || count > 96 || count % 3 != 0 {
            return Err(invalid());
        }
        let mut surface = color.draw_target()?;
        if surface.width > 64
            || surface.height > 64
            || required(self.origin)? != 0
            || required(self.scale)? != 1
            || required(self.targets)? != 1
            || required(self.mask)? != 0x1111
        {
            return Err(invalid());
        }
        for setting in self.disabled {
            if required(setting)? != 0 {
                return Err(invalid());
            }
        }
        let viewport = [
            surface.width as f32 / 2.,
            surface.height as f32 / 2.,
            1.,
            surface.width as f32 / 2.,
            surface.height as f32 / 2.,
            0.,
        ]
        .map(f32::to_bits);
        for (actual, expected) in self.viewport.iter().zip(viewport) {
            if required(*actual)? != expected {
                return Err(invalid());
            }
        }
        for (actual, expected) in
            self.clip
                .iter()
                .zip([surface.width << 16, surface.height << 16, 0, 1f32.to_bits()])
        {
            if required(*actual)? != expected {
                return Err(invalid());
            }
        }
        if required(self.attribute)? != 0x38200000 || required(self.stream[3])? != 0 {
            return Err(invalid());
        }
        for (i, p) in self.pipeline.iter().enumerate() {
            if i != 1 && i != 5 && required(p[0])? != 0 {
                return Err(invalid());
            }
        }
        let region = address(required(self.region[0])?, required(self.region[1])?)?;
        if region % 256 != 0 {
            return Err(invalid());
        }
        let mut pins = vec![];
        let mut program = |slot: usize, stage: Stage| -> Result<Program, E> {
            let p = self.pipeline[slot];
            if required(p[0])? != (1 | ((slot as u32) << 4)) {
                return Err(invalid());
            }
            let offset = u64::from(required(p[1])?);
            if offset % 256 != 0 {
                return Err(invalid());
            }
            let base = region.checked_add(offset).ok_or_else(invalid)?;
            let mut bytes = capture(space, GpuVa(base), 80, &mut pins)?;
            // Executable has no length field. Fetch whole bundles through the
            // first unconditional EXIT, bounded to the decoder's 2048 code bytes.
            let mut ended = false;
            for block in 0..64 {
                let bundle = capture(space, GpuVa(base + 80 + block * 32), 32, &mut pins)?;
                ended = bundle[8..]
                    .chunks_exact(8)
                    .any(|w| u64::from_le_bytes(w.try_into().expect("word")) == 0xe30000000007000f);
                bytes.extend(bundle);
                if ended {
                    break;
                }
            }
            if !ended {
                return Err(E::Shader("Missing EXIT within code budget".into()));
            }
            let shader = decode_program(&bytes, stage).map_err(|e| E::Shader(e.to_string()))?;
            let registers = required(p[2])?;
            if registers == 0 || registers > 255 || shader.required_registers() > registers as usize
            {
                return Err(invalid());
            }
            Ok(shader)
        };
        let vertex = program(1, Stage::Vertex)?;
        let fragment = program(5, Stage::Fragment)?;
        let extra = vertex.input_mask() & 0xf0 != 0;
        let stride = if extra { 32 } else { 16 };
        if required(self.stream[0])? != 0x1000 | stride as u32
            || extra && required(self.attribute1)? != 0x38200800
            || fragment.input_mask() != 0 && vertex.varying_outputs().is_none()
        {
            return Err(invalid());
        }
        let texture = if let Some(handle) = fragment.texture_handle() {
            if required(self.texture_mode)? != 1 {
                return Err(invalid());
            }
            let mut pools = [[0; 3]; 2];
            for (out, source) in pools.iter_mut().zip(self.texture_pools) {
                for (out, field) in out.iter_mut().zip(source) {
                    *out = required(field)?;
                }
            }
            Some(crate::sampled_texture::capture(
                space, pools, handle, &mut pins,
            )?)
        } else {
            None
        };
        let vertex_spirv = emit(&vertex).map_err(|e| E::Shader(e.to_string()))?;
        let fragment_spirv = emit(&fragment).map_err(|e| E::Shader(e.to_string()))?;
        surface.color = if fragment.input_mask() == 0 && texture.is_none() {
            fragment
                .evaluate([0; 4])
                .map_err(|e| E::Shader(e.to_string()))?
        } else {
            [0; 4]
        };
        if surface
            .color
            .iter()
            .any(|v| !f32::from_bits(*v).is_finite())
        {
            return Err(invalid());
        }
        let begin = address(required(self.stream[1])?, required(self.stream[2])?)?;
        let limit = address(required(self.limit[0])?, required(self.limit[1])?)?;
        let start = begin
            .checked_add((first * stride) as u64)
            .ok_or_else(invalid)?;
        let end = start
            .checked_add((count * stride) as u64)
            .ok_or_else(invalid)?;
        if begin % 4 != 0 || end == 0 || end - 1 > limit {
            return Err(invalid());
        }
        let bytes = capture(space, GpuVa(start), count * stride, &mut pins)?;
        let vertices = bytes
            .chunks_exact(stride)
            .map(|v| {
                std::array::from_fn(|c| {
                    u32::from_le_bytes(v[c * 4..c * 4 + 4].try_into().expect("word"))
                })
            })
            .collect();
        let varying_inputs = bytes
            .chunks_exact(stride)
            .map(|v| {
                if extra {
                    std::array::from_fn(|c| {
                        u32::from_le_bytes(v[16 + c * 4..20 + c * 4].try_into().expect("word"))
                    })
                } else {
                    [0; 4]
                }
            })
            .collect();
        let draw = Draw {
            surface,
            vertices,
            varying_inputs,
            texture,
            vertex,
            fragment,
            vertex_spirv,
            fragment_spirv,
        };
        // Validate vertex domain and raster bounds before exposing a plan.
        draw.raster()?;
        Ok((draw, pins))
    }
}
fn address(upper: u32, lower: u32) -> Result<u64, E> {
    if upper > 255 {
        return Err(invalid());
    }
    Ok((u64::from(upper) << 32) | u64::from(lower))
}
fn capture(
    space: &GpuAddressSpace,
    address: GpuVa,
    size: usize,
    pins: &mut Vec<GpuMemoryRange>,
) -> Result<Vec<u8>, E> {
    let pin = space.pin(address, size, Access::Read).map_err(E::Memory)?;
    let mut bytes = vec![0; size];
    pin.read(&mut bytes).map_err(E::Memory)?;
    pins.push(pin);
    Ok(bytes)
}
impl Draw {
    /// Fixed 1/16-pixel grid, pixel-center sampling and top-left edge ownership.
    /// Only finite, unclipped w=1 positions on this grid are accepted. Fragment
    /// attributes interpolate screen-linearly; depth/blending/culling are disabled.
    pub(crate) fn raster(&self) -> Result<Vec<Option<[u8; 4]>>, E> {
        let s = self.surface;
        if self.vertices.len() != self.varying_inputs.len() || s.width > 64 || s.height > 64 {
            return Err(invalid());
        }
        let mut points = vec![];
        let mut attributes = vec![];
        for (input, extra) in self.vertices.iter().zip(&self.varying_inputs) {
            let invocation = self
                .vertex
                .invoke([*input, *extra], None)
                .map_err(|e| E::Shader(e.to_string()))?;
            let p = invocation.output.map(f32::from_bits);
            let varying = invocation.varying.unwrap_or([0; 4]).map(f32::from_bits);
            if varying.iter().any(|x| !x.is_finite()) {
                return Err(invalid());
            }
            attributes.push(varying);
            if p.iter().any(|x| !x.is_finite())
                || p[3] != 1.
                || !(0.0..=1.0).contains(&p[2])
                || !(-1.0..=1.0).contains(&p[0])
                || !(-1.0..=1.0).contains(&p[1])
            {
                return Err(invalid());
            }
            let xy = [
                (p[0] + 1.) * (s.width as f32 / 2.) * 16.,
                (p[1] + 1.) * (s.height as f32 / 2.) * 16.,
            ];
            if xy.iter().any(|v| v.fract() != 0.) {
                return Err(invalid());
            }
            // A rounded viewport sum must not make an off-grid NDC value
            // appear aligned. Require an exact reversible grid position.
            if xy[0] / (s.width as f32 * 8.) - 1. != p[0]
                || xy[1] / (s.height as f32 * 8.) - 1. != p[1]
            {
                return Err(invalid());
            }
            points.push([xy[0] as i64, xy[1] as i64]);
        }
        let mut pixels = vec![None; (s.width * s.height) as usize];
        let uniform = self.fragment.input_mask() == 0 && self.texture.is_none();
        let constant = if uniform {
            Some(
                self.fragment
                    .evaluate([0; 4])
                    .map_err(|e| E::Shader(e.to_string()))?,
            )
        } else {
            None
        };
        for (triangle, varying) in points.chunks_exact(3).zip(attributes.chunks_exact(3)) {
            let [a, mut b, mut c] = [triangle[0], triangle[1], triangle[2]];
            let [av, mut bv, mut cv] = [varying[0], varying[1], varying[2]];
            let area = edge(a, b, c);
            if area == 0 {
                continue;
            }
            if area < 0 {
                std::mem::swap(&mut b, &mut c);
                std::mem::swap(&mut bv, &mut cv);
            }
            for y in s.rect[1]..s.rect[3] {
                for x in s.rect[0]..s.rect[2] {
                    let p = [i64::from(x) * 16 + 8, i64::from(y) * 16 + 8];
                    let inside = [(a, b), (b, c), (c, a)].into_iter().all(|(u, v)| {
                        let e = edge(u, v, p);
                        e > 0 || e == 0 && (v[1] < u[1] || v[1] == u[1] && v[0] > u[0])
                    });
                    if inside {
                        let color = if let Some(color) = constant {
                            color
                        } else {
                            let weights = [edge(b, c, p), edge(c, a, p), edge(a, b, p)];
                            let area = edge(a, b, c) as f64;
                            let input = std::array::from_fn(|i| {
                                ((weights[0] as f64 * av[i] as f64
                                    + weights[1] as f64 * bv[i] as f64
                                    + weights[2] as f64 * cv[i] as f64)
                                    / area) as f32
                            });
                            self.fragment
                                .invoke(
                                    [input.map(f32::to_bits), [0; 4]],
                                    self.texture.as_ref().map(|t| t.image()),
                                )
                                .map_err(|e| E::Shader(e.to_string()))?
                                .output
                        };
                        if color.iter().any(|x| !f32::from_bits(*x).is_finite()) {
                            return Err(invalid());
                        }
                        let mut surface = s;
                        surface.color = color;
                        pixels[(y * s.width + x) as usize] = Some(surface.storage_color().0);
                    }
                }
            }
        }
        Ok(pixels)
    }
}
fn edge(a: [i64; 2], b: [i64; 2], p: [i64; 2]) -> i64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
