//! Original bounded Maxwell SM53 shader subset, SSA bit/float IR and SPIR-V.
//! Raw instruction bundles only. Stage/interface metadata is supplied explicitly;
//! graphics program headers, branching, textures and shader memory are unsupported.
use std::fmt;
pub mod reference;
mod spirv;
pub use spirv::emit;

pub const MAX_CODE_BYTES: usize = 2048;
pub const MAX_SPIRV_WORDS: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Length,
    Budget,
    Schedule { offset: usize },
    Unsupported { offset: usize, word: u64 },
    Interface { offset: usize },
    UndefinedRegister { offset: usize, register: u8 },
    MissingExit,
    AfterExit { offset: usize },
    MissingOutput,
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
    instructions: usize,
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
    pub fn instruction_count(&self) -> usize {
        self.instructions
    }
    /// Independent serial reference semantics for an invocation. No GPU state.
    pub fn evaluate(&self, input: [u32; 4]) -> [u32; 4] {
        let mut bits = Vec::with_capacity(self.values.len());
        for value in &self.values {
            let x = match value.op {
                Op::Constant(x) => x,
                Op::Input(c) => input[c as usize],
                Op::BitsToFloat(a) | Op::FloatToBits(a) => bits[a],
                Op::Add(a, b) => (f32::from_bits(bits[a]) + f32::from_bits(bits[b])).to_bits(),
                Op::Multiply(a, b) => (f32::from_bits(bits[a]) * f32::from_bits(bits[b])).to_bits(),
            };
            bits.push(x);
        }
        self.outputs.map(|i| bits[i])
    }
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
                0x5c58000000070000 | 0x5c68000000070000
            ) {
                let lhs = read(&regs, a, offset)?;
                let rhs = read(&regs, b, offset)?;
                let lhs = value(&mut values, Type::F32, Op::BitsToFloat(lhs));
                let rhs = value(&mut values, Type::F32, Op::BitsToFloat(rhs));
                let op = if w >> 52 == 0x5c5 {
                    Op::Add(lhs, rhs)
                } else {
                    Op::Multiply(lhs, rhs)
                };
                let f = value(&mut values, Type::F32, op);
                Some(value(&mut values, Type::Bits32, Op::FloatToBits(f)))
            } else if matches!(
                w & !0x000180003ff000ff,
                0xefd87f800007ff00 | 0xeff07f800007ff00
            ) {
                let load = w >> 52 == 0xefd;
                let attr = ((w >> 20) & 0x3ff) as usize;
                let count = (((w >> 47) & 3) + 1) as usize;
                let base = if load { 0x80 } else { 0x70 };
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
                        regs[d as usize + c] =
                            Some(value(&mut values, Type::Bits32, Op::Input(component as u8)));
                    } else {
                        outputs[component] = Some(read(&regs, d + c as u8, offset)?);
                    }
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
    let [Some(x), Some(y), Some(z), Some(w)] = outputs else {
        return Err(Error::MissingOutput);
    };
    Ok(Program {
        stage,
        values,
        outputs: [x, y, z, w],
        instructions,
    })
}
