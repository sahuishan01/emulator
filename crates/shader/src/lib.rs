//! Original bounded Maxwell SM53 shader subset, SSA bit/float IR and SPIR-V.
//! Raw instruction bundles only. Stage/interface metadata is supplied explicitly;
//! bounded vertex varyings and one explicit-LOD 2D texture binding are supported.
use std::fmt;
pub mod census;
mod header;
pub mod maxwell;
pub mod reference;
mod spirv;
pub use header::{decode_program, HEADER_BYTES};
pub use spirv::{emit, emit_normalized_position};

pub const MAX_CODE_BYTES: usize = 2048;
pub const MAX_SPIRV_WORDS: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
    Compute,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Header,
    Length,
    Budget,
    Schedule { offset: usize },
    Unsupported { offset: usize, word: u64 },
    Interface { offset: usize },
    UndefinedRegister { offset: usize, register: u8 },
    MissingExit,
    AfterExit { offset: usize },
    MissingOutput,
    Invocation,
    TextureBinding,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Shader translation failed: {self:?}")
    }
}
impl std::error::Error for Error {}

/// Each value has an explicit type. Registers are bit patterns; arithmetic
/// consumes F32 bitcasts and creates F32 results, then bitcasts back to Bits32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Bits32,
    F32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Constant(u32),
    Input(u8),
    BitsToFloat(usize),
    FloatToBits(usize),
    Add(usize, usize),
    Multiply(usize, usize),
    Subtract(usize, usize),
    Fma(usize, usize, usize),
    FMin(usize, usize),
    FMax(usize, usize),
    Sin(usize),
    Cos(usize),
    Exp2(usize),
    Log2(usize),
    Rcp(usize),
    Rsqrt(usize),
    Sqrt(usize),
    IAdd(usize, usize),
    ISub(usize, usize),
    IMul(usize, usize),
    BitwiseAnd(usize, usize),
    BitwiseOr(usize, usize),
    BitwiseXor(usize, usize),
    ShiftLeft(usize, usize),
    ShiftRight(usize, usize),
    Sample {
        coordinates: [usize; 2],
        component: u8,
    },
    ConstantBuffer {
        bank: u8,
        offset: usize,
    },
    GlobalLoad {
        address: usize,
    },
    GlobalStore {
        address: usize,
        value: usize,
    },
    SharedLoad {
        offset: usize,
    },
    SharedStore {
        offset: usize,
        value: usize,
    },
    WorkgroupBarrier,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub(crate) ty: Type,
    pub(crate) op: Op,
}
impl Value {
    pub fn ty(&self) -> Type {
        self.ty
    }
    pub fn op(&self) -> &Op {
        &self.op
    }
}
/// Owned immutable plan: callers cannot construct invalid SSA or mutate a
/// translated module after validation. Output components refer to Bits32 values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub(crate) stage: Stage,
    pub(crate) values: Vec<Value>,
    pub(crate) outputs: [usize; 4],
    pub(crate) varying: Option<[usize; 4]>,
    pub(crate) input_mask: u8,
    pub(crate) texture_handle: Option<u32>,
    instructions: usize,
    required_registers: usize,
}
impl Program {
    pub fn stage(&self) -> Stage {
        self.stage
    }
    pub fn values(&self) -> &[Value] {
        &self.values
    }
    pub fn outputs(&self) -> [usize; 4] {
        self.outputs
    }
    pub fn required_registers(&self) -> usize {
        self.required_registers
    }
    pub fn instruction_count(&self) -> usize {
        self.instructions
    }
    pub fn input_mask(&self) -> u8 {
        self.input_mask
    }
    pub fn texture_handle(&self) -> Option<u32> {
        self.texture_handle
    }
    pub fn varying_outputs(&self) -> Option<[usize; 4]> {
        self.varying
    }
    /// Legacy single-vector invocation rejects shaders requiring extra inputs.
    pub fn evaluate(&self, input: [u32; 4]) -> Result<[u32; 4], Error> {
        if self.stage == Stage::Vertex && self.input_mask & 0xf0 != 0 {
            return Err(Error::Invocation);
        }
        Ok(self.invoke([input, [0; 4]], None)?.output)
    }
    /// Independent scalar semantics. Images are checked views, never GPU pointers.
    pub fn invoke(
        &self,
        input: [[u32; 4]; 2],
        image: Option<SampledImage<'_>>,
    ) -> Result<Invocation, Error> {
        if self.texture_handle.is_some() != image.is_some() {
            return Err(Error::TextureBinding);
        }
        if let Some(image) = image {
            image.validate()?;
        }
        let mut bits = Vec::with_capacity(self.values.len());
        for value in &self.values {
            let x = match value.op {
                Op::Constant(x) => x,
                Op::Input(c) => input[c as usize / 4][c as usize % 4],
                Op::BitsToFloat(a) | Op::FloatToBits(a) => bits[a],
                Op::Add(a, b) => (f32::from_bits(bits[a]) + f32::from_bits(bits[b])).to_bits(),
                Op::Subtract(a, b) => (f32::from_bits(bits[a]) - f32::from_bits(bits[b])).to_bits(),
                Op::Multiply(a, b) => (f32::from_bits(bits[a]) * f32::from_bits(bits[b])).to_bits(),
                Op::Fma(a, b, c) => (f32::from_bits(bits[a]) * f32::from_bits(bits[b])
                    + f32::from_bits(bits[c]))
                .to_bits(),
                Op::FMin(a, b) => {
                    let fa = f32::from_bits(bits[a]);
                    let fb = f32::from_bits(bits[b]);
                    fa.min(fb).to_bits()
                }
                Op::FMax(a, b) => {
                    let fa = f32::from_bits(bits[a]);
                    let fb = f32::from_bits(bits[b]);
                    fa.max(fb).to_bits()
                }
                Op::Sin(a) => (f32::from_bits(bits[a]).sin()).to_bits(),
                Op::Cos(a) => (f32::from_bits(bits[a]).cos()).to_bits(),
                Op::Exp2(a) => (f32::from_bits(bits[a]).exp2()).to_bits(),
                Op::Log2(a) => (f32::from_bits(bits[a]).log2()).to_bits(),
                Op::Rcp(a) => (1.0f32 / f32::from_bits(bits[a])).to_bits(),
                Op::Rsqrt(a) => (1.0f32 / f32::from_bits(bits[a]).sqrt()).to_bits(),
                Op::Sqrt(a) => (f32::from_bits(bits[a]).sqrt()).to_bits(),
                Op::IAdd(a, b) => bits[a].wrapping_add(bits[b]),
                Op::ISub(a, b) => bits[a].wrapping_sub(bits[b]),
                Op::IMul(a, b) => bits[a].wrapping_mul(bits[b]),
                Op::BitwiseAnd(a, b) => bits[a] & bits[b],
                Op::BitwiseOr(a, b) => bits[a] | bits[b],
                Op::BitwiseXor(a, b) => bits[a] ^ bits[b],
                Op::ShiftLeft(a, b) => bits[a].wrapping_shl(bits[b]),
                Op::ShiftRight(a, b) => bits[a].wrapping_shr(bits[b]),
                Op::ConstantBuffer { .. } => 0,
                Op::GlobalLoad { .. } => 0,
                Op::GlobalStore { .. } => 0,
                Op::SharedLoad { .. } => 0,
                Op::SharedStore { .. } => 0,
                Op::WorkgroupBarrier => 0,
                Op::Sample {
                    coordinates,
                    component,
                } => image
                    .ok_or(Error::TextureBinding)?
                    .sample(coordinates.map(|i| f32::from_bits(bits[i])))?[component as usize],
            };
            bits.push(x);
        }
        Ok(Invocation {
            output: self.outputs.map(|i| bits[i]),
            varying: self.varying.map(|v| v.map(|i| bits[i])),
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SampledImage<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
}
impl SampledImage<'_> {
    fn validate(self) -> Result<(), Error> {
        if self.width == 0
            || self.height == 0
            || self.width > 64
            || self.height > 64
            || self.pixels.len() != self.width as usize * self.height as usize * 4
        {
            return Err(Error::TextureBinding);
        }
        Ok(())
    }
    fn sample(self, uv: [f32; 2]) -> Result<[u32; 4], Error> {
        if uv.iter().any(|x| !x.is_finite()) {
            return Err(Error::Invocation);
        }
        let x = ((uv[0].clamp(0., 1.) * self.width as f32).floor() as u32).min(self.width - 1);
        let y = ((uv[1].clamp(0., 1.) * self.height as f32).floor() as u32).min(self.height - 1);
        let at = ((y * self.width + x) * 4) as usize;
        Ok(std::array::from_fn(|c| {
            (self.pixels[at + c] as f32 / 255.).to_bits()
        }))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub output: [u32; 4],
    pub varying: Option<[u32; 4]>,
}
fn value(values: &mut Vec<Value>, ty: Type, op: Op) -> usize {
    let id = values.len();
    values.push(Value { ty, op });
    id
}
fn read(regs: &[Option<usize>; 256], r: u8, offset: usize) -> Result<usize, Error> {
    regs[r as usize].ok_or(Error::UndefinedRegister {
        offset,
        register: r,
    })
}
/// Accept complete little-endian 32-byte bundles: scheduling word plus three
/// 64-bit instructions. Scheduling lower 63 bits are latency/reuse hints only
/// for this straight-line side-effect-free subset; bit63 is reserved and fails.
/// PT only, no modifiers except complete MOV lanes. EXIT must be unconditional;
/// all remaining slots must be the exact NOP. No hidden zeroed register state.
pub fn decode(code: &[u8], stage: Stage) -> Result<Program, Error> {
    if code.len() > MAX_CODE_BYTES {
        return Err(Error::Budget);
    }
    if code.is_empty() || code.len() % 32 != 0 {
        return Err(Error::Length);
    }
    let mut values = Vec::new();
    let zero = value(&mut values, Type::Bits32, Op::Constant(0));
    let mut regs = [None; 256];
    regs[255] = Some(zero);
    let mut outputs = [None; 4];
    let mut varying = [None; 4];
    let mut texture_handle = None;
    let mut input_mask = 0u8;
    let mut exit = false;
    let mut instructions = 0;
    for (bundle, bytes) in code.chunks_exact(32).enumerate() {
        if u64::from_le_bytes(bytes[..8].try_into().expect("word")) >> 63 != 0 {
            return Err(Error::Schedule {
                offset: bundle * 32,
            });
        }
        for slot in 1..4 {
            let offset = bundle * 32 + slot * 8;
            let w = u64::from_le_bytes(bytes[slot * 8..slot * 8 + 8].try_into().expect("word"));
            if w == 0x50b0000000070f00 {
                continue;
            }
            if exit {
                return Err(Error::AfterExit { offset });
            }
            instructions += 1;
            let d = w as u8;
            let a = (w >> 8) as u8;
            let b = (w >> 20) as u8;
            let out = if w & !0x000ffffffff000ff == 0x010000000007f000 {
                // MOV32I, all four lanes, PT. Preserve every immediate bit.
                Some(value(
                    &mut values,
                    Type::Bits32,
                    Op::Constant((w >> 20) as u32),
                ))
            } else if w & !0x000000000ff000ff == 0x5c98078000070000 {
                Some(read(&regs, b, offset)?)
            } else if matches!(
                w & !0x000000000ff0ffff,
                0x5c58000000070000 | 0x5c68000000070000 | 0x5c50000000070000
            ) {
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                let lhs = value(&mut values, Type::F32, Op::BitsToFloat(lhs));
                let rhs = value(&mut values, Type::F32, Op::BitsToFloat(rhs));
                let op = if (w >> 52) & 0xff == 0x5c && (w >> 20) & 0x0f == 0 {
                    Op::Subtract(lhs, rhs)
                } else if w >> 52 == 0x5c5 {
                    Op::Add(lhs, rhs)
                } else {
                    Op::Multiply(lhs, rhs)
                };
                let f = value(&mut values, Type::F32, op);
                Some(value(&mut values, Type::Bits32, Op::FloatToBits(f)))
            } else if w & !0x000000000ff0ffff == 0x5980000000070000 {
                // FFMA: d = a * b + c
                let c_reg = ((w >> 39) & 0xff) as u8;
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                let acc = read(&regs, c_reg, offset)?;
                let lhs = value(&mut values, Type::F32, Op::BitsToFloat(lhs));
                let rhs = value(&mut values, Type::F32, Op::BitsToFloat(rhs));
                let acc = value(&mut values, Type::F32, Op::BitsToFloat(acc));
                let f = value(&mut values, Type::F32, Op::Fma(lhs, rhs, acc));
                Some(value(&mut values, Type::Bits32, Op::FloatToBits(f)))
            } else if w & !0x000000000ff0ffff == 0x5080000000070000 {
                // MUFU: transcendental function
                let src = read(&regs, a, offset)?;
                let src_f = value(&mut values, Type::F32, Op::BitsToFloat(src));
                let sub_op = ((w >> 20) & 0x0f) as u8;
                let op = match sub_op {
                    0 => Op::Cos(src_f),
                    1 => Op::Sin(src_f),
                    2 => Op::Exp2(src_f),
                    3 => Op::Log2(src_f),
                    4 => Op::Rcp(src_f),
                    5 => Op::Rsqrt(src_f),
                    _ => Op::Sqrt(src_f),
                };
                let f = value(&mut values, Type::F32, op);
                Some(value(&mut values, Type::Bits32, Op::FloatToBits(f)))
            } else if w & !0x000000000ff0ffff == 0x5c10000000070000 {
                // IADD: d = a + b
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                Some(value(&mut values, Type::Bits32, Op::IAdd(lhs, rhs)))
            } else if w & !0x000000000ff0ffff == 0x5c11000000070000 {
                // ISUB: d = a - b
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                Some(value(&mut values, Type::Bits32, Op::ISub(lhs, rhs)))
            } else if w & !0x000000000ff0ffff == 0x5c20000000070000 {
                // IMUL: d = a * b
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                Some(value(&mut values, Type::Bits32, Op::IMul(lhs, rhs)))
            } else if w & !0x000000000ff0ffff == 0x5c40000000070000 {
                // LOP: bitwise logic
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                let logic_op = ((w >> 39) & 3) as u8;
                let op = match logic_op {
                    0 => Op::BitwiseAnd(lhs, rhs),
                    1 => Op::BitwiseOr(lhs, rhs),
                    _ => Op::BitwiseXor(lhs, rhs),
                };
                Some(value(&mut values, Type::Bits32, op))
            } else if w & !0x000000000ff0ffff == 0x5c48000000070000 {
                // SHL: shift left
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                Some(value(&mut values, Type::Bits32, Op::ShiftLeft(lhs, rhs)))
            } else if w & !0x000000000ff0ffff == 0x5c4c000000070000 {
                // SHR: shift right
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                Some(value(&mut values, Type::Bits32, Op::ShiftRight(lhs, rhs)))
            } else if w & !0x000000000000ffff == 0x50b8000000070000 {
                // BAR.SYNC: workgroup execution barrier
                Some(value(&mut values, Type::Bits32, Op::WorkgroupBarrier))
            } else if w & !0x000000000ff0ffff == 0xef90000000070000 {
                // LDC: constant buffer read c[bank][offset]
                let bank = ((w >> 24) & 0x1f) as u8;
                let offset_val = (w >> 8) as usize;
                Some(value(
                    &mut values,
                    Type::Bits32,
                    Op::ConstantBuffer {
                        bank,
                        offset: offset_val,
                    },
                ))
            } else if matches!(
                w & !0x000180003ff000ff,
                0xefd87f800007ff00 | 0xeff07f800007ff00
            ) {
                let load = w >> 52 == 0xefd;
                let attr = ((w >> 20) & 0x3ff) as usize;
                let count = (((w >> 47) & 3) + 1) as usize;
                let base = if load {
                    if attr >= 0x90 {
                        0x90
                    } else {
                        0x80
                    }
                } else if attr >= 0x80 {
                    0x80
                } else {
                    0x70
                };
                if stage != Stage::Vertex
                    || attr % 4 != 0
                    || attr < base
                    || attr + count * 4 > base + 16
                    || d as usize + count > 255
                {
                    return Err(Error::Interface { offset });
                }
                // Capture all sources before writes, as for other SSA operands.
                for c in 0..count {
                    let component = (attr - base) / 4 + c;
                    if load {
                        let component = component + if base == 0x90 { 4 } else { 0 };
                        input_mask |= 1 << component;
                        regs[d as usize + c] =
                            Some(value(&mut values, Type::Bits32, Op::Input(component as u8)));
                    } else if base == 0x80 {
                        varying[component] = Some(read(&regs, d + c as u8, offset)?);
                    } else {
                        outputs[component] = Some(read(&regs, d + c as u8, offset)?);
                    }
                }
                None
            } else if w & !0x0000003ff00000ff == 0xe003ff800ff7ff00 {
                // IPA.PASS, center sample, direct generic0, RZ multiplier.
                let attr = ((w >> 28) & 0x3ff) as usize;
                if stage != Stage::Fragment || !(0x80..0x90).contains(&attr) || attr % 4 != 0 {
                    return Err(Error::Interface { offset });
                }
                let c = (attr - 0x80) / 4;
                input_mask |= 1 << c;
                Some(value(&mut values, Type::Bits32, Op::Input(c as u8)))
            } else if w & !0x000000000ff0ffff == 0xdeb80027a0070000 {
                // TEX.B.LZ, 2D, all RGBA, no modifiers, static bindless handle.
                if stage != Stage::Fragment || d > 251 || a == 255 {
                    return Err(Error::Interface { offset });
                }
                let coordinates = [read(&regs, a, offset)?, read(&regs, a + 1, offset)?];
                let handle = read(&regs, b, offset)?;
                let Op::Constant(handle) = values[handle].op else {
                    return Err(Error::TextureBinding);
                };
                if texture_handle.is_some_and(|old| old != handle) {
                    return Err(Error::TextureBinding);
                }
                texture_handle = Some(handle);
                for c in 0..4 {
                    regs[d as usize + c] = Some(value(
                        &mut values,
                        Type::Bits32,
                        Op::Sample {
                            coordinates,
                            component: c as u8,
                        },
                    ));
                }
                None
            } else if w == 0xe30000000007000f {
                exit = true;
                if stage == Stage::Fragment {
                    outputs.copy_from_slice(&regs[..4]);
                }
                None
            } else {
                return Err(Error::Unsupported { offset, word: w });
            };
            if let Some(id) = out {
                if d != 255 {
                    regs[d as usize] = Some(id);
                }
            }
        }
    }
    if !exit {
        return Err(Error::MissingExit);
    }
    let (resolved_outputs, varying) = if stage == Stage::Compute {
        ([zero, zero, zero, zero], None)
    } else {
        let [Some(x), Some(y), Some(z), Some(w)] = outputs else {
            return Err(Error::MissingOutput);
        };
        let varying = if varying.iter().all(Option::is_none) {
            None
        } else {
            let [Some(x), Some(y), Some(z), Some(w)] = varying else {
                return Err(Error::MissingOutput);
            };
            Some([x, y, z, w])
        };
        ([x, y, z, w], varying)
    };
    Ok(Program {
        stage,
        values,
        outputs: resolved_outputs,
        varying,
        input_mask,
        texture_handle,
        instructions,
        required_registers: regs[..255]
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |r| r + 1),
    })
}
