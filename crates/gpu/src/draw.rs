//! Original bounded B197 non-indexed triangle and shader input preparation.
use crate::{
    clear::ColorRegisters, ColorClear, ExecutionError as E, GpuAddressSpace, GpuMemoryRange, GpuVa,
};
use emulator_memory::Access;
use emulator_shader::{decode_program, emit, Program, Stage};

/// Owned host-independent draw. Surface color is the constant fragment result;
/// vertices are captured input bits, before evaluating the guest vertex shader.
#[derive(Clone, Debug)]
pub struct Draw {
    pub surface: ColorClear,
    pub vertices: Vec<[u32; 4]>,
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
        if required(self.attribute)? != 0x38200000
            || required(self.stream[0])? != 0x1010
            || required(self.stream[3])? != 0
        {
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
        let vertex_spirv = emit(&vertex).map_err(|e| E::Shader(e.to_string()))?;
        let fragment_spirv = emit(&fragment).map_err(|e| E::Shader(e.to_string()))?;
        surface.color = fragment.evaluate([0; 4]);
        if surface
            .color
            .iter()
            .any(|v| !f32::from_bits(*v).is_finite())
        {
            return Err(invalid());
        }
        let begin = address(required(self.stream[1])?, required(self.stream[2])?)?;
        let limit = address(required(self.limit[0])?, required(self.limit[1])?)?;
        let start = begin.checked_add((first * 16) as u64).ok_or_else(invalid)?;
        let end = start.checked_add((count * 16) as u64).ok_or_else(invalid)?;
        if begin % 4 != 0 || end == 0 || end - 1 > limit {
            return Err(invalid());
        }
        let bytes = capture(space, GpuVa(start), count * 16, &mut pins)?;
        let vertices = bytes
            .chunks_exact(16)
            .map(|v| {
                std::array::from_fn(|c| {
                    u32::from_le_bytes(v[c * 4..c * 4 + 4].try_into().expect("word"))
                })
            })
            .collect();
        let draw = Draw {
            surface,
            vertices,
            vertex,
            fragment,
            vertex_spirv,
            fragment_spirv,
        };
        // Validate vertex domain and raster bounds before exposing a plan.
        draw.coverage()?;
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
    /// output is uniform and depth/blending/culling are explicitly disabled.
    pub(crate) fn coverage(&self) -> Result<Vec<bool>, E> {
        let s = self.surface;
        let mut points = vec![];
        for input in &self.vertices {
            let p = self.vertex.evaluate(*input).map(f32::from_bits);
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
        let mut coverage = vec![false; (s.width * s.height) as usize];
        for triangle in points.chunks_exact(3) {
            let [a, mut b, mut c] = [triangle[0], triangle[1], triangle[2]];
            let area = edge(a, b, c);
            if area == 0 {
                continue;
            }
            if area < 0 {
                std::mem::swap(&mut b, &mut c);
            }
            for y in s.rect[1]..s.rect[3] {
                for x in s.rect[0]..s.rect[2] {
                    let p = [i64::from(x) * 16 + 8, i64::from(y) * 16 + 8];
                    let inside = [(a, b), (b, c), (c, a)].into_iter().all(|(u, v)| {
                        let e = edge(u, v, p);
                        e > 0 || e == 0 && (v[1] < u[1] || v[1] == u[1] && v[0] > u[0])
                    });
                    coverage[(y * s.width + x) as usize] |= inside;
                }
            }
        }
        Ok(coverage)
    }
}
fn edge(a: [i64; 2], b: [i64; 2], p: [i64; 2]) -> i64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
