//! Original A64 interpreter subset. This is not a complete Arm or Switch implementation.

use emulator_memory::{AddressSpace, MemoryError};
use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nzcv {
    pub negative: bool,
    pub zero: bool,
    pub carry: bool,
    pub overflow: bool,
}

impl Nzcv {
    fn holds(self, condition: u8) -> bool {
        let result = match condition >> 1 {
            0 => self.zero,
            1 => self.carry,
            2 => self.negative,
            3 => self.overflow,
            4 => self.carry && !self.zero,
            5 => self.negative == self.overflow,
            6 => !self.zero && self.negative == self.overflow,
            7 => true,
            _ => unreachable!("condition comes from a four-bit instruction field"),
        };
        if condition & 1 != 0 && condition != 15 {
            !result
        } else {
            result
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Width {
    W32,
    X64,
}

impl Width {
    fn mask(self) -> u64 {
        match self {
            Self::W32 => u32::MAX as u64,
            Self::X64 => u64::MAX,
        }
    }
    fn sign(self) -> u64 {
        match self {
            Self::W32 => 1 << 31,
            Self::X64 => 1 << 63,
        }
    }
    fn bytes(self) -> u64 {
        match self {
            Self::W32 => 4,
            Self::X64 => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Instruction {
    Nop,
    ReadThreadLocal {
        rd: u8,
    },
    Movz {
        width: Width,
        rd: u8,
        value: u64,
    },
    MoveWide {
        width: Width,
        rd: u8,
        value: u64,
        mask: u64,
        operation: u8,
    },
    PcRelative {
        rd: u8,
        displacement: i64,
        page: bool,
    },
    AddSub {
        width: Width,
        rd: u8,
        rn: u8,
        immediate: u64,
        subtract: bool,
        flags: bool,
    },
    Branch {
        displacement: i64,
        link: bool,
    },
    ConditionalBranch {
        displacement: i64,
        condition: u8,
    },
    CompareBranch {
        width: Width,
        rt: u8,
        displacement: i64,
        nonzero: bool,
    },
    Return {
        rn: u8,
    },
    LoadStore {
        width: Width,
        rt: u8,
        rn: u8,
        offset: u64,
        load: bool,
    },
    SupervisorCall {
        immediate: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionError {
    Memory(MemoryError),
    UnsupportedInstruction { pc: u64, word: u32 },
    ReservedEncoding { pc: u64, word: u32 },
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(error) => error.fmt(f),
            Self::UnsupportedInstruction { pc, word } => {
                write!(f, "unsupported A64 instruction {word:#010x} at {pc:#x}")
            }
            Self::ReservedEncoding { pc, word } => {
                write!(f, "reserved A64 encoding {word:#010x} at {pc:#x}")
            }
        }
    }
}

impl std::error::Error for ExecutionError {}

impl From<MemoryError> for ExecutionError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepEvent {
    Continue,
    SupervisorCall { immediate: u16 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cpu {
    pub x: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: Nzcv,
    pub tpidrro_el0: u64,
}

impl Cpu {
    pub fn new(entry: u64, stack_pointer: u64) -> Self {
        Self {
            pc: entry,
            sp: stack_pointer,
            ..Self::default()
        }
    }

    fn read_zero(&self, index: u8) -> u64 {
        if index == 31 {
            0
        } else {
            self.x[usize::from(index)]
        }
    }

    fn read_sp(&self, index: u8) -> u64 {
        if index == 31 {
            self.sp
        } else {
            self.x[usize::from(index)]
        }
    }

    fn write_zero(&mut self, index: u8, value: u64) {
        if index != 31 {
            self.x[usize::from(index)] = value;
        }
    }

    fn write_sp(&mut self, index: u8, value: u64) {
        if index == 31 {
            self.sp = value;
        } else {
            self.x[usize::from(index)] = value;
        }
    }

    pub fn step(&mut self, memory: &mut AddressSpace) -> Result<StepEvent, ExecutionError> {
        let word = memory.fetch_u32(self.pc)?;
        let instruction = decode(self.pc, word)?;
        let mut next_pc = self.pc.wrapping_add(4);
        let mut event = StepEvent::Continue;
        match instruction {
            Instruction::Nop => {}
            Instruction::ReadThreadLocal { rd } => self.write_zero(rd, self.tpidrro_el0),
            Instruction::Movz { width, rd, value } => self.write_zero(rd, value & width.mask()),
            Instruction::MoveWide {
                width,
                rd,
                value,
                mask,
                operation,
            } => {
                let result = match operation {
                    0 => !value,
                    3 => (self.read_zero(rd) & !mask) | value,
                    _ => unreachable!("decoder admits MOVN or MOVK"),
                } & width.mask();
                self.write_zero(rd, result);
            }
            Instruction::PcRelative {
                rd,
                displacement,
                page,
            } => {
                let base = if page { self.pc & !0xfff } else { self.pc };
                self.write_zero(rd, base.wrapping_add_signed(displacement));
            }
            Instruction::AddSub {
                width,
                rd,
                rn,
                immediate,
                subtract,
                flags,
            } => {
                let lhs = self.read_sp(rn) & width.mask();
                let rhs = immediate & width.mask();
                let result = if subtract {
                    lhs.wrapping_sub(rhs)
                } else {
                    lhs.wrapping_add(rhs)
                } & width.mask();
                if flags {
                    self.nzcv = Nzcv {
                        negative: result & width.sign() != 0,
                        zero: result == 0,
                        carry: if subtract {
                            lhs >= rhs
                        } else {
                            (lhs as u128 + rhs as u128) > width.mask() as u128
                        },
                        overflow: if subtract {
                            ((lhs ^ rhs) & (lhs ^ result) & width.sign()) != 0
                        } else {
                            (!(lhs ^ rhs) & (lhs ^ result) & width.sign()) != 0
                        },
                    };
                    self.write_zero(rd, result);
                } else {
                    self.write_sp(rd, result);
                }
            }
            Instruction::Branch { displacement, link } => {
                if link {
                    self.x[30] = next_pc;
                }
                next_pc = self.pc.wrapping_add_signed(displacement);
            }
            Instruction::ConditionalBranch {
                displacement,
                condition,
            } => {
                if self.nzcv.holds(condition) {
                    next_pc = self.pc.wrapping_add_signed(displacement);
                }
            }
            Instruction::CompareBranch {
                width,
                rt,
                displacement,
                nonzero,
            } => {
                if ((self.read_zero(rt) & width.mask()) != 0) == nonzero {
                    next_pc = self.pc.wrapping_add_signed(displacement);
                }
            }
            Instruction::Return { rn } => next_pc = self.read_zero(rn),
            Instruction::LoadStore {
                width,
                rt,
                rn,
                offset,
                load,
            } => {
                let address = self.read_sp(rn).wrapping_add(offset);
                if load {
                    let value = match width {
                        Width::W32 => memory.read_u32(address)? as u64,
                        Width::X64 => memory.read_u64(address)?,
                    };
                    self.write_zero(rt, value);
                } else {
                    let value = self.read_zero(rt);
                    match width {
                        Width::W32 => memory.write_u32(address, value as u32)?,
                        Width::X64 => memory.write_u64(address, value)?,
                    }
                }
            }
            Instruction::SupervisorCall { immediate } => {
                event = StepEvent::SupervisorCall { immediate }
            }
        }
        self.pc = next_pc;
        Ok(event)
    }
}

fn decode(pc: u64, word: u32) -> Result<Instruction, ExecutionError> {
    let width = if word & (1 << 31) == 0 {
        Width::W32
    } else {
        Width::X64
    };
    let rd = (word & 31) as u8;
    let rn = ((word >> 5) & 31) as u8;
    if word == 0xd503_201f {
        return Ok(Instruction::Nop);
    }
    if word & 0xffff_ffe0 == 0xd53b_d060 {
        return Ok(Instruction::ReadThreadLocal { rd });
    }
    if word & 0x1f00_0000 == 0x1000_0000 {
        let immediate = ((word >> 5) & 0x7ffff) << 2 | ((word >> 29) & 3);
        let page = word & (1 << 31) != 0;
        return Ok(Instruction::PcRelative {
            rd,
            displacement: signed_field(immediate, 21) * if page { 4096 } else { 1 },
            page,
        });
    }
    if word & 0x7f80_0000 == 0x1280_0000 || word & 0x7f80_0000 == 0x7280_0000 {
        let shift = ((word >> 21) & 3) * 16;
        if width == Width::W32 && shift >= 32 {
            return Err(ExecutionError::ReservedEncoding { pc, word });
        }
        return Ok(Instruction::MoveWide {
            width,
            rd,
            value: u64::from((word >> 5) & 0xffff) << shift,
            mask: 0xffff_u64 << shift,
            operation: ((word >> 29) & 3) as u8,
        });
    }
    if word & 0x7f80_0000 == 0x5280_0000 {
        let shift = ((word >> 21) & 3) * 16;
        if width == Width::W32 && shift >= 32 {
            return Err(ExecutionError::ReservedEncoding { pc, word });
        }
        return Ok(Instruction::Movz {
            width,
            rd,
            value: u64::from((word >> 5) & 0xffff) << shift,
        });
    }
    if word & 0x1f80_0000 == 0x1100_0000 {
        return Ok(Instruction::AddSub {
            width,
            rd,
            rn,
            immediate: u64::from((word >> 10) & 0xfff)
                << if word & (1 << 22) != 0 { 12 } else { 0 },
            subtract: word & (1 << 30) != 0,
            flags: word & (1 << 29) != 0,
        });
    }
    if word & 0x7c00_0000 == 0x1400_0000 {
        return Ok(Instruction::Branch {
            displacement: signed_field(word & 0x03ff_ffff, 26) * 4,
            link: word & (1 << 31) != 0,
        });
    }
    if word & 0xff00_0010 == 0x5400_0000 {
        return Ok(Instruction::ConditionalBranch {
            displacement: signed_field((word >> 5) & 0x7ffff, 19) * 4,
            condition: (word & 15) as u8,
        });
    }
    if word & 0x7e00_0000 == 0x3400_0000 {
        return Ok(Instruction::CompareBranch {
            width,
            rt: rd,
            displacement: signed_field((word >> 5) & 0x7ffff, 19) * 4,
            nonzero: word & (1 << 24) != 0,
        });
    }
    if word & 0xffff_fc1f == 0xd65f_0000 {
        return Ok(Instruction::Return { rn });
    }
    if word & 0xbfc0_0000 == 0xb940_0000 || word & 0xbfc0_0000 == 0xb900_0000 {
        let width = if word & (1 << 30) != 0 {
            Width::X64
        } else {
            Width::W32
        };
        return Ok(Instruction::LoadStore {
            width,
            rt: rd,
            rn,
            offset: u64::from((word >> 10) & 0xfff) * width.bytes(),
            load: word & (1 << 22) != 0,
        });
    }
    if word & 0xffe0_001f == 0xd400_0001 {
        return Ok(Instruction::SupervisorCall {
            immediate: ((word >> 5) & 0xffff) as u16,
        });
    }
    Err(ExecutionError::UnsupportedInstruction { pc, word })
}

fn signed_field(value: u32, bits: u32) -> i64 {
    ((u64::from(value) << (64 - bits)) as i64) >> (64 - bits)
}
