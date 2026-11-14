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
    fn bits(self) -> u8 {
        match self {
            Self::W32 => 32,
            Self::X64 => 64,
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
    AddSubRegister {
        width: Width,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: u8,
        amount: u8,
        subtract: bool,
        flags: bool,
    },
    LogicalRegister {
        width: Width,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: u8,
        amount: u8,
        op: u8, // 0 = AND, 1 = BIC, 2 = ORR, 3 = ORN, 4 = EOR, 5 = EON, 6 = ANDS, 7 = BICS
    },
    Bitfield {
        width: Width,
        rd: u8,
        rn: u8,
        immr: u8,
        imms: u8,
        signed: bool,
    },
    ExclusiveLoadStore {
        width: Width,
        rt: u8,
        rn: u8,
        rs: u8, // status register for STXR
        load: bool,
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
    LoadStorePair {
        width: Width,
        rt1: u8,
        rt2: u8,
        rn: u8,
        offset: i64,
        load: bool,
        pre_index: bool,
        post_index: bool,
    },
    SupervisorCall {
        immediate: u16,
    },
    FpData2 {
        double: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        op: u8,
    },
    FpData1 {
        double: bool,
        rd: u8,
        rn: u8,
        op: u8,
    },
    FpCompare {
        double: bool,
        rn: u8,
        rm: u8,
        compare_zero: bool,
    },
    FpMultiplyAdd {
        double: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
        sub: bool,
        neg: bool,
    },
    FpConvertInt {
        sf: bool,
        double: bool,
        rd: u8,
        rn: u8,
        op: u8,
    },
    FpLoadStore {
        size: u8,
        rt: u8,
        rn: u8,
        offset: u64,
        load: bool,
    },
    FpLoadStorePair {
        size: u8,
        rt1: u8,
        rt2: u8,
        rn: u8,
        offset: i64,
        load: bool,
        pre_index: bool,
        post_index: bool,
    },
    VectorBitwise {
        rd: u8,
        rn: u8,
        rm: u8,
        op: u8,
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
    pub v: [u128; 32],
    pub fpcr: u32,
    pub fpsr: u32,
}

impl Cpu {
    pub fn new(entry: u64, stack_pointer: u64) -> Self {
        Self {
            pc: entry,
            sp: stack_pointer,
            ..Self::default()
        }
    }

    pub fn read_v_u32(&self, index: u8) -> u32 {
        self.v[usize::from(index)] as u32
    }

    pub fn read_v_f32(&self, index: u8) -> f32 {
        f32::from_bits(self.read_v_u32(index))
    }

    pub fn read_v_u64(&self, index: u8) -> u64 {
        self.v[usize::from(index)] as u64
    }

    pub fn read_v_f64(&self, index: u8) -> f64 {
        f64::from_bits(self.read_v_u64(index))
    }

    pub fn read_v_u128(&self, index: u8) -> u128 {
        self.v[usize::from(index)]
    }

    pub fn write_v_u32(&mut self, index: u8, value: u32) {
        self.v[usize::from(index)] = u128::from(value);
    }

    pub fn write_v_f32(&mut self, index: u8, value: f32) {
        self.write_v_u32(index, value.to_bits());
    }

    pub fn write_v_u64(&mut self, index: u8, value: u64) {
        self.v[usize::from(index)] = u128::from(value);
    }

    pub fn write_v_f64(&mut self, index: u8, value: f64) {
        self.write_v_u64(index, value.to_bits());
    }

    pub fn write_v_u128(&mut self, index: u8, value: u128) {
        self.v[usize::from(index)] = value;
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
            Instruction::AddSubRegister {
                width,
                rd,
                rn,
                rm,
                shift,
                amount,
                subtract,
                flags,
            } => {
                let lhs = self.read_zero(rn) & width.mask();
                let operand = self.read_zero(rm) & width.mask();
                let rhs = match shift {
                    0 => operand << amount,
                    1 => operand >> amount,
                    2 if width == Width::W32 => ((operand as u32 as i32) >> amount) as u32 as u64,
                    2 => ((operand as i64) >> amount) as u64,
                    _ => unreachable!("validated shift"),
                } & width.mask();
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
                    self.write_zero(rd, result);
                }
            }
            Instruction::LogicalRegister {
                width,
                rd,
                rn,
                rm,
                shift,
                amount,
                op,
            } => {
                let lhs = self.read_zero(rn) & width.mask();
                let operand = self.read_zero(rm) & width.mask();
                let mut rhs = match shift {
                    0 => operand << amount,
                    1 => operand >> amount,
                    2 if width == Width::W32 => ((operand as u32 as i32) >> amount) as u32 as u64,
                    2 => ((operand as i64) >> amount) as u64,
                    // ROR
                    3 => {
                        let bits = width.bits();
                        let a = amount % bits;
                        if a == 0 { operand } else { (operand >> a) | (operand << (bits - a)) }
                    }
                    _ => unreachable!("validated shift"),
                } & width.mask();
                // Inverted operand for BIC, ORN, EON, BICS
                if op & 1 != 0 {
                    rhs = (!rhs) & width.mask();
                }
                let result = match op >> 1 {
                    0 | 3 => lhs & rhs, // AND / ANDS / BIC / BICS
                    1 => lhs | rhs,     // ORR / ORN
                    2 => lhs ^ rhs,     // EOR / EON
                    _ => unreachable!(),
                } & width.mask();
                if op >= 6 {
                    // ANDS / BICS set flags
                    self.nzcv = Nzcv {
                        negative: result & width.sign() != 0,
                        zero: result == 0,
                        carry: false,
                        overflow: false,
                    };
                }
                self.write_zero(rd, result);
            }
            Instruction::Bitfield {
                width,
                rd,
                rn,
                immr,
                imms,
                signed,
            } => {
                let n = width.bits();
                let src = self.read_zero(rn) & width.mask();
                // ROR(src, immr)
                let ror = if immr == 0 {
                    src
                } else {
                    let r = immr % n;
                    (src >> r) | ((src << (n - r)) & width.mask())
                };
                let wmask = if imms >= immr {
                    let len = (imms - immr + 1) as u32;
                    if len == 64 { u64::MAX } else { (1u64 << len) - 1 }
                } else {
                    let len = (imms + 1) as u32;
                    (1u64 << len) - 1
                };
                let mut result = ror & wmask;
                if signed && (imms >= immr || imms < immr) {
                    let sign_bit = (imms.wrapping_sub(immr) % n) as u32;
                    let sign_mask = 1u64 << sign_bit;
                    if result & sign_mask != 0 {
                        // Sign extend from sign_bit to top of width
                        let top_mask = if sign_bit >= 63 { 0 } else { (!0u64) << (sign_bit + 1) };
                        result |= top_mask & width.mask();
                    }
                }
                self.write_zero(rd, result & width.mask());
            }
            Instruction::ExclusiveLoadStore {
                width,
                rt,
                rn,
                rs,
                load,
            } => {
                let address = self.read_sp(rn);
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
                    // In single-threaded guest interpreter model, exclusive store always succeeds (status 0)
                    self.write_zero(rs, 0);
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
            Instruction::LoadStorePair {
                width,
                rt1,
                rt2,
                rn,
                offset,
                load,
                pre_index,
                post_index,
            } => {
                let base = self.read_sp(rn);
                let address = if pre_index {
                    base.wrapping_add_signed(offset)
                } else {
                    base
                };
                let second_address = address.wrapping_add(width.bytes());
                if load {
                    let v1 = match width {
                        Width::W32 => memory.read_u32(address)? as u64,
                        Width::X64 => memory.read_u64(address)?,
                    };
                    let v2 = match width {
                        Width::W32 => memory.read_u32(second_address)? as u64,
                        Width::X64 => memory.read_u64(second_address)?,
                    };
                    self.write_zero(rt1, v1);
                    self.write_zero(rt2, v2);
                } else {
                    let v1 = self.read_zero(rt1);
                    let v2 = self.read_zero(rt2);
                    match width {
                        Width::W32 => {
                            memory.write_u32(address, v1 as u32)?;
                            memory.write_u32(second_address, v2 as u32)?;
                        }
                        Width::X64 => {
                            memory.write_u64(address, v1)?;
                            memory.write_u64(second_address, v2)?;
                        }
                    }
                }
                if pre_index {
                    self.write_sp(rn, address);
                } else if post_index {
                    self.write_sp(rn, base.wrapping_add_signed(offset));
                }
            }
            Instruction::SupervisorCall { immediate } => {
                event = StepEvent::SupervisorCall { immediate }
            }
            Instruction::FpData2 { double, rd, rn, rm, op } => {
                if double {
                    let a = self.read_v_f64(rn);
                    let b = self.read_v_f64(rm);
                    let res = match op {
                        0 => a * b,
                        1 => a / b,
                        2 => a + b,
                        3 => a - b,
                        4 => a.max(b),
                        5 => a.min(b),
                        6 => if a.is_nan() { b } else if b.is_nan() { a } else { a.max(b) },
                        7 => if a.is_nan() { b } else if b.is_nan() { a } else { a.min(b) },
                        8 => -(a * b),
                        _ => 0.0,
                    };
                    self.write_v_f64(rd, res);
                } else {
                    let a = self.read_v_f32(rn);
                    let b = self.read_v_f32(rm);
                    let res = match op {
                        0 => a * b,
                        1 => a / b,
                        2 => a + b,
                        3 => a - b,
                        4 => a.max(b),
                        5 => a.min(b),
                        6 => if a.is_nan() { b } else if b.is_nan() { a } else { a.max(b) },
                        7 => if a.is_nan() { b } else if b.is_nan() { a } else { a.min(b) },
                        8 => -(a * b),
                        _ => 0.0,
                    };
                    self.write_v_f32(rd, res);
                }
            }
            Instruction::FpData1 { double, rd, rn, op } => {
                if double {
                    let a = self.read_v_f64(rn);
                    match op {
                        0 => self.write_v_f64(rd, a),
                        1 => self.write_v_f64(rd, a.abs()),
                        2 => self.write_v_f64(rd, -a),
                        3 => self.write_v_f64(rd, a.sqrt()),
                        4 | 5 => self.write_v_f32(rd, a as f32),
                        _ => {}
                    }
                } else {
                    let a = self.read_v_f32(rn);
                    match op {
                        0 => self.write_v_f32(rd, a),
                        1 => self.write_v_f32(rd, a.abs()),
                        2 => self.write_v_f32(rd, -a),
                        3 => self.write_v_f32(rd, a.sqrt()),
                        4 | 5 => self.write_v_f64(rd, f64::from(a)),
                        _ => {}
                    }
                }
            }
            Instruction::FpCompare { double, rn, rm, compare_zero } => {
                if double {
                    let a = self.read_v_f64(rn);
                    let b = if compare_zero { 0.0 } else { self.read_v_f64(rm) };
                    if a.is_nan() || b.is_nan() {
                        self.nzcv = Nzcv { negative: false, zero: false, carry: true, overflow: true };
                    } else if a == b {
                        self.nzcv = Nzcv { negative: false, zero: true, carry: true, overflow: false };
                    } else if a < b {
                        self.nzcv = Nzcv { negative: true, zero: false, carry: false, overflow: false };
                    } else {
                        self.nzcv = Nzcv { negative: false, zero: false, carry: true, overflow: false };
                    }
                } else {
                    let a = self.read_v_f32(rn);
                    let b = if compare_zero { 0.0 } else { self.read_v_f32(rm) };
                    if a.is_nan() || b.is_nan() {
                        self.nzcv = Nzcv { negative: false, zero: false, carry: true, overflow: true };
                    } else if a == b {
                        self.nzcv = Nzcv { negative: false, zero: true, carry: true, overflow: false };
                    } else if a < b {
                        self.nzcv = Nzcv { negative: true, zero: false, carry: false, overflow: false };
                    } else {
                        self.nzcv = Nzcv { negative: false, zero: false, carry: true, overflow: false };
                    }
                }
            }
            Instruction::FpMultiplyAdd { double, rd, rn, rm, ra, sub, neg } => {
                if double {
                    let a = self.read_v_f64(ra);
                    let n = self.read_v_f64(rn);
                    let m = self.read_v_f64(rm);
                    let prod = n * m;
                    let res = match (sub, neg) {
                        (false, false) => a + prod,
                        (true, false) => a - prod,
                        (false, true) => -a - prod,
                        (true, true) => -a + prod,
                    };
                    self.write_v_f64(rd, res);
                } else {
                    let a = self.read_v_f32(ra);
                    let n = self.read_v_f32(rn);
                    let m = self.read_v_f32(rm);
                    let prod = n * m;
                    let res = match (sub, neg) {
                        (false, false) => a + prod,
                        (true, false) => a - prod,
                        (false, true) => -a - prod,
                        (true, true) => -a + prod,
                    };
                    self.write_v_f32(rd, res);
                }
            }
            Instruction::FpConvertInt { sf, double, rd, rn, op } => {
                match op {
                    0 => {
                        let val: i64 = if double {
                            self.read_v_f64(rn) as i64
                        } else {
                            self.read_v_f32(rn) as i64
                        };
                        let res = if sf { val as u64 } else { (val as u32) as u64 };
                        self.write_zero(rd, res);
                    }
                    1 => {
                        let val: u64 = if double {
                            self.read_v_f64(rn) as u64
                        } else {
                            self.read_v_f32(rn) as u64
                        };
                        let res = if sf { val } else { (val as u32) as u64 };
                        self.write_zero(rd, res);
                    }
                    2 => {
                        let int_val: i64 = if sf {
                            self.read_zero(rn) as i64
                        } else {
                            (self.read_zero(rn) as i32) as i64
                        };
                        if double {
                            self.write_v_f64(rd, int_val as f64);
                        } else {
                            self.write_v_f32(rd, int_val as f32);
                        }
                    }
                    3 => {
                        let uint_val: u64 = if sf {
                            self.read_zero(rn)
                        } else {
                            self.read_zero(rn) as u32 as u64
                        };
                        if double {
                            self.write_v_f64(rd, uint_val as f64);
                        } else {
                            self.write_v_f32(rd, uint_val as f32);
                        }
                    }
                    6 => {
                        if sf && double {
                            self.write_v_u64(rd, self.read_zero(rn));
                        } else if !sf && !double {
                            self.write_v_u32(rd, self.read_zero(rn) as u32);
                        }
                    }
                    7 => {
                        if sf && double {
                            self.write_zero(rd, self.read_v_u64(rn));
                        } else if !sf && !double {
                            self.write_zero(rd, self.read_v_u32(rn) as u64);
                        }
                    }
                    _ => {}
                }
            }
            Instruction::FpLoadStore { size, rt, rn, offset, load } => {
                let address = self.read_sp(rn).wrapping_add(offset);
                if load {
                    match size {
                        2 => {
                            let val = memory.read_u32(address)?;
                            self.write_v_u32(rt, val);
                        }
                        3 => {
                            let val = memory.read_u64(address)?;
                            self.write_v_u64(rt, val);
                        }
                        4 => {
                            let low = memory.read_u64(address)?;
                            let high = memory.read_u64(address.wrapping_add(8))?;
                            self.write_v_u128(rt, (u128::from(high) << 64) | u128::from(low));
                        }
                        _ => {}
                    }
                } else {
                    match size {
                        2 => memory.write_u32(address, self.read_v_u32(rt))?,
                        3 => memory.write_u64(address, self.read_v_u64(rt))?,
                        4 => {
                            let val = self.read_v_u128(rt);
                            let low = val as u64;
                            let high = (val >> 64) as u64;
                            memory.write_u64(address, low)?;
                            memory.write_u64(address.wrapping_add(8), high)?;
                        }
                        _ => {}
                    }
                }
            }
            Instruction::FpLoadStorePair { size, rt1, rt2, rn, offset, load, pre_index, post_index } => {
                let base = self.read_sp(rn);
                let address = if pre_index { base.wrapping_add_signed(offset) } else { base };
                let elem_bytes: u64 = match size {
                    2 => 4,
                    3 => 8,
                    4 => 16,
                    _ => 0,
                };
                let second_address = address.wrapping_add(elem_bytes);
                if load {
                    match size {
                        2 => {
                            let v1 = memory.read_u32(address)?;
                            let v2 = memory.read_u32(second_address)?;
                            self.write_v_u32(rt1, v1);
                            self.write_v_u32(rt2, v2);
                        }
                        3 => {
                            let v1 = memory.read_u64(address)?;
                            let v2 = memory.read_u64(second_address)?;
                            self.write_v_u64(rt1, v1);
                            self.write_v_u64(rt2, v2);
                        }
                        4 => {
                            let low1 = memory.read_u64(address)?;
                            let high1 = memory.read_u64(address.wrapping_add(8))?;
                            let low2 = memory.read_u64(second_address)?;
                            let high2 = memory.read_u64(second_address.wrapping_add(8))?;
                            self.write_v_u128(rt1, (u128::from(high1) << 64) | u128::from(low1));
                            self.write_v_u128(rt2, (u128::from(high2) << 64) | u128::from(low2));
                        }
                        _ => {}
                    }
                } else {
                    match size {
                        2 => {
                            memory.write_u32(address, self.read_v_u32(rt1))?;
                            memory.write_u32(second_address, self.read_v_u32(rt2))?;
                        }
                        3 => {
                            memory.write_u64(address, self.read_v_u64(rt1))?;
                            memory.write_u64(second_address, self.read_v_u64(rt2))?;
                        }
                        4 => {
                            let v1 = self.read_v_u128(rt1);
                            let v2 = self.read_v_u128(rt2);
                            memory.write_u64(address, v1 as u64)?;
                            memory.write_u64(address.wrapping_add(8), (v1 >> 64) as u64)?;
                            memory.write_u64(second_address, v2 as u64)?;
                            memory.write_u64(second_address.wrapping_add(8), (v2 >> 64) as u64)?;
                        }
                        _ => {}
                    }
                }
                if pre_index {
                    self.write_sp(rn, address);
                } else if post_index {
                    self.write_sp(rn, base.wrapping_add_signed(offset));
                }
            }
            Instruction::VectorBitwise { rd, rn, rm, op } => {
                let a = self.read_v_u128(rn);
                let b = self.read_v_u128(rm);
                let res = match op {
                    0 => a & b,
                    1 => a & (!b),
                    2 => a | b,
                    3 => a | (!b),
                    4 => a ^ b,
                    _ => 0,
                };
                self.write_v_u128(rd, res);
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
        let is_adrp = word & (1 << 31) != 0;
        let immlo = (word >> 29) & 3;
        let immhi = (word >> 5) & 0x7_ffff;
        let imm21 = (immhi << 2) | immlo;
        let displacement = signed_field(imm21, 21) * if is_adrp { 4096 } else { 1 };
        return Ok(Instruction::PcRelative {
            rd,
            displacement,
            page: is_adrp,
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
    if word & 0x1f20_0000 == 0x0b00_0000 {
        let shift = ((word >> 22) & 3) as u8;
        let amount = ((word >> 10) & 63) as u8;
        if shift == 3 || (width == Width::W32 && amount >= 32) {
            return Err(ExecutionError::ReservedEncoding { pc, word });
        }
        return Ok(Instruction::AddSubRegister {
            width,
            rd,
            rn,
            rm: ((word >> 16) & 31) as u8,
            shift,
            amount,
            subtract: word & (1 << 30) != 0,
            flags: word & (1 << 29) != 0,
        });
    }
    // Logical (shifted register): AND, BIC, ORR, ORN, EOR, EON, ANDS, BICS
    // bits 28..24 must be 01010
    if word & 0x1f00_0000 == 0x0a00_0000 {
        let opc = ((word >> 29) & 3) as u8;
        let n = ((word >> 21) & 1) as u8;
        let shift = ((word >> 22) & 3) as u8;
        let amount = ((word >> 10) & 63) as u8;
        if width == Width::W32 && amount >= 32 {
            return Err(ExecutionError::ReservedEncoding { pc, word });
        }
        let op = (opc << 1) | n;
        return Ok(Instruction::LogicalRegister {
            width,
            rd,
            rn,
            rm: ((word >> 16) & 31) as u8,
            shift,
            amount,
            op,
        });
    }
    // Bitfield instructions: SBFM (opc=00), BFM (opc=01), UBFM (opc=10)
    // 31: sf, 30-29: opc, 28-23: 100110, 22: N, 21-16: immr, 15-10: imms, 9-5: Rn, 4-0: Rd
    if word & 0x1f80_0000 == 0x1300_0000 {
        let opc = (word >> 29) & 3;
        let n_bit = (word >> 22) & 1;
        // In ARMv8, for 32-bit (sf=0), N must be 0 and immr/imms < 32
        // For 64-bit (sf=1), N must be 1
        if (width == Width::W32 && n_bit != 0) || (width == Width::X64 && n_bit == 0) {
            return Err(ExecutionError::ReservedEncoding { pc, word });
        }
        let immr = ((word >> 16) & 63) as u8;
        let imms = ((word >> 10) & 63) as u8;
        if width == Width::W32 && (immr >= 32 || imms >= 32) {
            return Err(ExecutionError::ReservedEncoding { pc, word });
        }
        if opc == 0 || opc == 2 {
            // SBFM (0) or UBFM (2)
            return Ok(Instruction::Bitfield {
                width,
                rd,
                rn,
                immr,
                imms,
                signed: opc == 0,
            });
        }
    }
    // Exclusive load/store: LDXR / STXR
    // LDXR: 31: size[1], 30: size[0], 29-24: 001000, 23: 0, 22: L=1, 21: 0, 20-16: Rs (11111 for ldxr), 15-10: 011111, 9-5: Rn, 4-0: Rt
    // STXR: 31: size[1], 30: size[0], 29-24: 001000, 23: 0, 22: L=0, 21: 0, 20-16: Rs, 15-10: 011111, 9-5: Rn, 4-0: Rt
    // In ARMv8 load/store exclusive:
    // size field is bits [31:30]: 10 is 32-bit word, 11 is 64-bit xword.
    // bit 22 is L (1 for load, 0 for store), bit 23 is 0, bit 21 is 0.
    // Mask out bits 31-30 (size) and bit 22 (L):
    // 0x3fe0_fc00 has bit 22 set (0x400000). To ignore bit 22: 0x3fa0_fc00
    if word & 0x3fa0_fc00 == 0x0800_7c00 {
        let size = (word >> 30) & 3;
        if size == 2 || size == 3 {
            let width = if size == 3 { Width::X64 } else { Width::W32 };
            let is_load = (word & (1 << 22)) != 0;
            let rs = ((word >> 16) & 31) as u8;
            let rt = (word & 31) as u8;
            let rn = ((word >> 5) & 31) as u8;
            return Ok(Instruction::ExclusiveLoadStore {
                width,
                rt,
                rn,
                rs,
                load: is_load,
            });
        }
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
    // LDP / STP (Load/Store Pair)
    // opc: bits [31:30] (00 = 32-bit, 10 = 64-bit)
    // V: bit 26 (0 = general-purpose registers)
    // L: bit 22 (1 = load, 0 = store)
    // mode: bits [24:23] (01 = post-index, 10 = signed offset, 11 = pre-index)
    if word & 0x3a00_0000 == 0x2800_0000 && word & (1 << 26) == 0 {
        let opc = (word >> 30) & 3;
        let mode = (word >> 23) & 3;
        if (opc == 0 || opc == 2) && mode != 0 {
            let width = if opc == 2 { Width::X64 } else { Width::W32 };
            let imm7 = signed_field((word >> 15) & 0x7f, 7);
            let offset = imm7 * (width.bytes() as i64);
            let rt1 = (word & 31) as u8;
            let rt2 = ((word >> 10) & 31) as u8;
            return Ok(Instruction::LoadStorePair {
                width,
                rt1,
                rt2,
                rn,
                offset,
                load: word & (1 << 22) != 0,
                pre_index: mode == 3,
                post_index: mode == 1,
            });
        }
    }
    // Floating-Point Data-Processing (2 source): FADD, FSUB, FMUL, FDIV, FMAX, FMIN, FMAXNM, FMINNM, FNMUL
    if word & 0x5f20_0c00 == 0x1e20_0800 {
        let fptype = (word >> 22) & 3;
        if fptype <= 1 {
            let double = fptype == 1;
            let rm = ((word >> 16) & 31) as u8;
            let op = ((word >> 12) & 0xF) as u8;
            let rn = ((word >> 5) & 31) as u8;
            let rd = (word & 31) as u8;
            if op <= 8 {
                return Ok(Instruction::FpData2 {
                    double,
                    rd,
                    rn,
                    rm,
                    op,
                });
            }
        }
    }

    // Floating-Point Compare: FCMP / FCMPE
    if word & 0x5f20_fc17 == 0x1e20_2000 {
        let fptype = (word >> 22) & 3;
        if fptype <= 1 {
            let double = fptype == 1;
            let compare_zero = (word & (1 << 3)) != 0;
            let rm = ((word >> 16) & 31) as u8;
            let rn = ((word >> 5) & 31) as u8;
            return Ok(Instruction::FpCompare {
                double,
                rn,
                rm,
                compare_zero,
            });
        }
    }

    // Floating-Point Data-Processing (1 source): FMOV, FABS, FNEG, FSQRT, FCVT
    if word & 0x5f20_7c00 == 0x1e20_4000 {
        let fptype = (word >> 22) & 3;
        if fptype <= 1 {
            let double = fptype == 1;
            let op = ((word >> 15) & 0x3F) as u8;
            let rn = ((word >> 5) & 31) as u8;
            let rd = (word & 31) as u8;
            if op <= 5 {
                return Ok(Instruction::FpData1 {
                    double,
                    rd,
                    rn,
                    op,
                });
            }
        }
    }

    // Floating-Point Multiply-Add: FMADD, FMSUB, FNMADD, FNMSUB
    if word & 0x5f00_0000 == 0x1f00_0000 {
        let fptype = (word >> 22) & 3;
        if fptype <= 1 {
            let double = fptype == 1;
            let sub = (word & (1 << 21)) != 0;
            let rm = ((word >> 16) & 31) as u8;
            let neg = (word & (1 << 15)) != 0;
            let ra = ((word >> 10) & 31) as u8;
            let rn = ((word >> 5) & 31) as u8;
            let rd = (word & 31) as u8;
            return Ok(Instruction::FpMultiplyAdd {
                double,
                rd,
                rn,
                rm,
                ra,
                sub,
                neg,
            });
        }
    }

    // Conversion between FP and Integer / FMOV
    if word & 0x5f20_fc00 == 0x1e20_0000 {
        let sf = (word >> 31) & 1 != 0;
        let fptype = (word >> 22) & 3;
        if fptype <= 1 {
            let double = fptype == 1;
            let op = ((word >> 16) & 7) as u8;
            let rn = ((word >> 5) & 31) as u8;
            let rd = (word & 31) as u8;
            return Ok(Instruction::FpConvertInt {
                sf,
                double,
                rd,
                rn,
                op,
            });
        }
    }

    // SIMD & FP Load / Store (unsigned immediate offset)
    // S: size=2, D: size=3, Q: size=0 with opc=10/11
    if word & 0x3f00_0000 == 0x3d00_0000 {
        let size_bits = (word >> 30) & 3;
        let opc = (word >> 22) & 3;
        let (size, scale) = if size_bits == 0 && (opc == 2 || opc == 3) {
            (4u8, 16u64)
        } else if size_bits == 2 {
            (2u8, 4u64)
        } else if size_bits == 3 {
            (3u8, 8u64)
        } else {
            (0u8, 0u64)
        };
        if size != 0 {
            let imm12 = u64::from((word >> 10) & 0xfff);
            let offset = imm12 * scale;
            let load = if size == 4 { opc == 3 } else { opc == 1 };
            return Ok(Instruction::FpLoadStore {
                size,
                rt: rd,
                rn,
                offset,
                load,
            });
        }
    }

    // SIMD & FP Load / Store Pair (LDP / STP)
    if word & 0x3a00_0000 == 0x2800_0000 && word & (1 << 26) != 0 {
        let opc = (word >> 30) & 3;
        let mode = (word >> 23) & 3;
        let (size, scale) = match opc {
            0 => (2u8, 4i64),
            1 => (3u8, 8i64),
            2 => (4u8, 16i64),
            _ => (0u8, 0i64),
        };
        if size != 0 && mode != 0 {
            let imm7 = signed_field((word >> 15) & 0x7f, 7);
            let offset = imm7 * scale;
            let rt1 = (word & 31) as u8;
            let rt2 = ((word >> 10) & 31) as u8;
            return Ok(Instruction::FpLoadStorePair {
                size,
                rt1,
                rt2,
                rn,
                offset,
                load: (word & (1 << 22)) != 0,
                pre_index: mode == 3,
                post_index: mode == 1,
            });
        }
    }

    // Vector Bitwise (128-bit NEON: AND, BIC, ORR, ORN, EOR)
    if word & 0xbf20_fc00 == 0x0ea0_1c00
        || word & 0xbf20_fc00 == 0x2ea0_1c00
        || word & 0xbf20_fc00 == 0x0e20_1c00
        || word & 0xbf20_fc00 == 0x0e60_1c00
        || word & 0xbf20_fc00 == 0x0ee0_1c00
        || word & 0xbf20_fc00 == 0x2e20_1c00
    {
        let op = if word & 0xbf20_fc00 == 0x0e20_1c00 {
            0
        } else if word & 0xbf20_fc00 == 0x0e60_1c00 {
            1
        } else if word & 0xbf20_fc00 == 0x0ea0_1c00 || word & 0xbf20_fc00 == 0x2ea0_1c00 {
            2
        } else if word & 0xbf20_fc00 == 0x0ee0_1c00 {
            3
        } else {
            4
        };
        return Ok(Instruction::VectorBitwise {
            rd,
            rn,
            rm: ((word >> 16) & 31) as u8,
            op,
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
