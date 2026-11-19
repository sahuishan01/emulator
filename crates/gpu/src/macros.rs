//! Original Maxwell 3D macro method expander (MME) model.
//!
//! The 3D class exposes a small macro processor: guests upload instruction
//! words (LOAD_MME_INSTRUCTION_RAM*), a start-address table
//! (LOAD_MME_START_ADDRESS_RAM*), then call macro `n` by writing its first
//! parameter to `0x3800 + 8n` and further parameters to `0x3804 + 8n`. A
//! macro reads parameters and 3D registers and emits method writes.
//!
//! This model expands macro calls into plain method writes before execution,
//! so the method executor never sees MME methods. It keeps a 3D register file
//! (last written values) for macro register reads. Behaviour follows the
//! publicly documented MME instruction format; no vendor or emulator code is
//! used. Unknown encodings, parameter underflow, runaway macros and replay
//! shadow mode fail explicitly.
use crate::{ExecutionError, MethodWrite};

const LOAD_INSTRUCTION_POINTER: u32 = 0x114;
const LOAD_INSTRUCTION: u32 = 0x118;
const LOAD_START_POINTER: u32 = 0x11c;
const LOAD_START: u32 = 0x120;
const SHADOW_RAM_CONTROL: u32 = 0x124;
/// First macro call method (byte offset); 128 macros, two methods each.
pub const MACRO_BASE: u32 = 0x3800;
const MACRO_END: u32 = 0x4000;
/// Instruction RAM words and start-address table entries.
pub const INSTRUCTION_WORDS: usize = 0x2000;
pub const MACRO_COUNT: usize = 0x80;
/// 3D register file words visible to macro reads (below the macro range).
const REGISTER_WORDS: usize = (MACRO_BASE / 4) as usize;
/// Instructions one call may execute before it is treated as runaway.
pub const STEP_LIMIT: usize = 1 << 20;
/// Writes one submit may produce through macros.
const EMIT_LIMIT: usize = 1 << 21;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroError {
    InstructionPointer(u32),
    StartPointer(u32),
    UnloadedMacro(u32),
    ProgramCounter(u32),
    ParameterUnderflow { macro_index: u32, pc: u32 },
    UnsupportedOperation { pc: u32, word: u32 },
    BranchInDelaySlot { pc: u32 },
    ReadOutOfRange { pc: u32, register: u32 },
    SendOutOfRange { pc: u32, method: u32 },
    StepLimit(u32),
    EmitLimit,
    ParameterWithoutCall(u32),
    ShadowReplay,
}

#[derive(Clone, Debug)]
pub(crate) struct MacroState {
    /// Subchannels bound to the 3D class.
    threed: [bool; 8],
    code: Box<[u32]>,
    starts: Box<[Option<u32>]>,
    instruction_pointer: u32,
    start_pointer: u32,
    registers: Box<[u32]>,
}
impl Default for MacroState {
    fn default() -> Self {
        Self {
            threed: [false; 8],
            code: vec![0; INSTRUCTION_WORDS].into(),
            starts: vec![None; MACRO_COUNT].into(),
            instruction_pointer: 0,
            start_pointer: 0,
            registers: vec![0; REGISTER_WORDS].into(),
        }
    }
}

struct PendingCall {
    subchannel: u8,
    index: u32,
    parameters: Vec<u32>,
}

impl MacroState {
    /// Expand MME uploads and macro calls on 3D subchannels. Everything else
    /// passes through in order. The state is updated in place; callers keep
    /// a copy for rollback.
    pub(crate) fn expand(
        &mut self,
        writes: &[MethodWrite],
    ) -> Result<Vec<MethodWrite>, ExecutionError> {
        let mut out = Vec::with_capacity(writes.len());
        let mut pending: Option<PendingCall> = None;
        for &write in writes {
            let sub = usize::from(write.subchannel);
            let offset = write.method_byte_offset();
            if offset == 0 {
                self.threed[sub] = write.data == crate::MAXWELL_B;
            }
            if !self.threed[sub] || offset == 0 {
                self.finish(&mut pending, &mut out)?;
                out.push(write);
                continue;
            }
            if (MACRO_BASE..MACRO_END).contains(&offset) {
                let index = (offset - MACRO_BASE) / 8;
                if offset % 8 == 0 {
                    self.finish(&mut pending, &mut out)?;
                    pending = Some(PendingCall {
                        subchannel: write.subchannel,
                        index,
                        parameters: vec![write.data],
                    });
                } else {
                    match pending.as_mut() {
                        Some(call) if call.subchannel == write.subchannel => {
                            call.parameters.push(write.data);
                        }
                        _ => return Err(macro_error(MacroError::ParameterWithoutCall(offset))),
                    }
                }
                continue;
            }
            self.finish(&mut pending, &mut out)?;
            self.write_register(write, &mut out)?;
        }
        self.finish(&mut pending, &mut out)?;
        Ok(out)
    }

    fn finish(
        &mut self,
        pending: &mut Option<PendingCall>,
        out: &mut Vec<MethodWrite>,
    ) -> Result<(), ExecutionError> {
        let Some(call) = pending.take() else {
            return Ok(());
        };
        let emitted = self.run(&call).map_err(macro_error)?;
        for write in emitted {
            self.write_register(write, out)?;
        }
        Ok(())
    }

    /// Handle a plain (non-call) 3D write: MME upload methods are consumed,
    /// other writes update the register file and are forwarded.
    fn write_register(
        &mut self,
        write: MethodWrite,
        out: &mut Vec<MethodWrite>,
    ) -> Result<(), ExecutionError> {
        let offset = write.method_byte_offset();
        match offset {
            LOAD_INSTRUCTION_POINTER => self.instruction_pointer = write.data,
            LOAD_INSTRUCTION => {
                let slot =
                    self.code
                        .get_mut(self.instruction_pointer as usize)
                        .ok_or(macro_error(MacroError::InstructionPointer(
                            self.instruction_pointer,
                        )))?;
                *slot = write.data;
                self.instruction_pointer += 1;
            }
            LOAD_START_POINTER => self.start_pointer = write.data,
            LOAD_START => {
                let slot = self
                    .starts
                    .get_mut(self.start_pointer as usize)
                    .ok_or(macro_error(MacroError::StartPointer(self.start_pointer)))?;
                *slot = Some(write.data);
                self.start_pointer += 1;
            }
            SHADOW_RAM_CONTROL => {
                // 0 track, 1 track with filter, 2 passthrough: register reads
                // return the last written value. Replay (3) is not modelled.
                if write.data & 3 == 3 {
                    return Err(macro_error(MacroError::ShadowReplay));
                }
            }
            _ => {
                if let Some(slot) = self.registers.get_mut((offset / 4) as usize) {
                    *slot = write.data;
                }
                if out.len() >= EMIT_LIMIT {
                    return Err(macro_error(MacroError::EmitLimit));
                }
                out.push(write);
            }
        }
        Ok(())
    }

    /// Interpret one macro call. Register reads see every earlier write of
    /// the stream, including this macro's own sends (the shadow RAM tracks
    /// writes as they happen).
    fn run(&mut self, call: &PendingCall) -> Result<Vec<MethodWrite>, MacroError> {
        let start =
            self.starts[call.index as usize].ok_or(MacroError::UnloadedMacro(call.index))?;
        let code = &self.code;
        let mut machine = Machine {
            registers: &mut self.registers,
            regs: [0; 8],
            carry: false,
            method: 0,
            parameters: &call.parameters,
            next_parameter: 1,
            index: call.index,
            subchannel: call.subchannel,
            emitted: Vec::new(),
        };
        machine.regs[1] = call.parameters[0];
        let mut pc = start;
        let mut delay: Option<u32> = None; // branch target after the delay slot
        let mut exiting = false;
        for _ in 0..STEP_LIMIT {
            let word = *code
                .get(pc as usize)
                .ok_or(MacroError::ProgramCounter(pc))?;
            let in_delay_slot = delay.is_some() || exiting;
            let mut next = pc.wrapping_add(1);
            let mut exit_bit = word & 0x80 != 0;
            match word & 7 {
                7 => {
                    if in_delay_slot {
                        return Err(MacroError::BranchInDelaySlot { pc });
                    }
                    let value = machine.regs[field(word, 11, 3) as usize];
                    let if_not_zero = word & 0x10 != 0;
                    let taken = (value != 0) == if_not_zero;
                    if taken {
                        let target = pc.wrapping_add(immediate(word) as u32);
                        exit_bit = false;
                        if word & 0x20 != 0 {
                            next = target; // no delay slot
                        } else {
                            // A taken branch does not exit; its exit bit
                            // only ends a loop that falls through (NVN uses
                            // "loop while rN != 0, else exit").
                            delay = Some(target);
                            pc = next;
                            continue;
                        }
                    }
                }
                op => {
                    let result = machine.operate(pc, word, op)?;
                    machine.assign(pc, word, result)?;
                }
            }
            if let Some(target) = delay.take() {
                next = target;
            }
            if exiting {
                return Ok(machine.emitted);
            }
            if exit_bit {
                exiting = true;
            }
            pc = next;
        }
        Err(MacroError::StepLimit(call.index))
    }
}

struct Machine<'a> {
    registers: &'a mut [u32],
    regs: [u32; 8],
    carry: bool,
    /// Method address register: bits 0..12 method word, 12..18 increment.
    method: u32,
    parameters: &'a [u32],
    next_parameter: usize,
    index: u32,
    subchannel: u8,
    emitted: Vec<MethodWrite>,
}
impl Machine<'_> {
    fn operate(&mut self, pc: u32, word: u32, op: u32) -> Result<u32, MacroError> {
        let a = self.regs[field(word, 11, 3) as usize];
        let b = self.regs[field(word, 14, 3) as usize];
        let src_bit = field(word, 17, 5);
        let size = field(word, 22, 5);
        let dst_bit = field(word, 27, 5);
        let mask = if size == 0 {
            0
        } else {
            u32::MAX >> (32 - size)
        };
        Ok(match op {
            0 => {
                let (value, carry) = match field(word, 17, 5) {
                    0 => add(a, b, false),
                    1 => add(a, b, self.carry),
                    2 => add(a, !b, true),
                    3 => add(a, !b, self.carry),
                    8 => (a ^ b, self.carry),
                    9 => (a | b, self.carry),
                    10 => (a & b, self.carry),
                    11 => (a & !b, self.carry),
                    12 => (!(a & b), self.carry),
                    _ => return Err(MacroError::UnsupportedOperation { pc, word }),
                };
                self.carry = carry;
                value
            }
            1 => a.wrapping_add(immediate(word) as u32),
            2 => (a & !(mask << dst_bit)) | (((b >> src_bit) & mask) << dst_bit),
            3 => b
                .checked_shr(a)
                .map_or(0, |v| (v & mask).checked_shl(dst_bit).unwrap_or(0)),
            4 => (b.checked_shr(src_bit).unwrap_or(0) & mask)
                .checked_shl(a)
                .unwrap_or(0),
            5 => {
                let register = a.wrapping_add(immediate(word) as u32);
                *self
                    .registers
                    .get(register as usize)
                    .ok_or(MacroError::ReadOutOfRange { pc, register })?
            }
            _ => return Err(MacroError::UnsupportedOperation { pc, word }),
        })
    }

    fn fetch(&mut self, pc: u32) -> Result<u32, MacroError> {
        let value =
            *self
                .parameters
                .get(self.next_parameter)
                .ok_or(MacroError::ParameterUnderflow {
                    macro_index: self.index,
                    pc,
                })?;
        self.next_parameter += 1;
        Ok(value)
    }

    fn send(&mut self, pc: u32, value: u32) -> Result<(), MacroError> {
        let method = self.method & 0xfff;
        if method * 4 >= MACRO_BASE {
            return Err(MacroError::SendOutOfRange { pc, method });
        }
        if let Some(slot) = self.registers.get_mut(method as usize) {
            *slot = value;
        }
        self.emitted.push(MethodWrite {
            subchannel: self.subchannel,
            method_word: method as u16,
            data: value,
        });
        if self.emitted.len() > EMIT_LIMIT {
            return Err(MacroError::EmitLimit);
        }
        let increment = (self.method >> 12) & 0x3f;
        self.method = (self.method & !0xfff) | ((method + increment) & 0xfff);
        Ok(())
    }

    fn assign(&mut self, pc: u32, word: u32, result: u32) -> Result<(), MacroError> {
        let dst = field(word, 8, 3) as usize;
        let value = match field(word, 4, 3) {
            0 => self.fetch(pc)?,
            1 => result,
            2 => {
                self.method = result;
                result
            }
            3 => {
                let fetched = self.fetch(pc)?;
                self.send(pc, result)?;
                fetched
            }
            4 => {
                self.send(pc, result)?;
                result
            }
            5 => {
                self.method = result;
                self.fetch(pc)?
            }
            6 => {
                let fetched = self.fetch(pc)?;
                self.method = result;
                self.send(pc, (result >> 12) & 0x3f)?;
                fetched
            }
            _ => {
                self.method = result;
                self.send(pc, (result >> 12) & 0x3f)?;
                result
            }
        };
        if dst != 0 {
            self.regs[dst] = value;
        }
        Ok(())
    }
}

fn field(word: u32, shift: u32, bits: u32) -> u32 {
    (word >> shift) & ((1 << bits) - 1)
}
/// Signed 18-bit immediate in bits 14..32.
fn immediate(word: u32) -> i32 {
    (word as i32) >> 14
}
fn add(a: u32, b: u32, carry_in: bool) -> (u32, bool) {
    let sum = u64::from(a) + u64::from(b) + u64::from(carry_in);
    (sum as u32, sum >> 32 != 0)
}
fn macro_error(error: MacroError) -> ExecutionError {
    ExecutionError::Macro(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDI: u32 = 1;
    const MOVE: u32 = 1;
    const SET_METHOD: u32 = 2;
    const SEND: u32 = 4;
    const FETCH: u32 = 0;
    const EXIT: u32 = 0x80;

    fn imm(assign: u32, dst: u32, a: u32, value: i32) -> u32 {
        ADDI | assign << 4 | dst << 8 | a << 11 | ((value as u32) << 14)
    }
    fn alu(assign: u32, dst: u32, a: u32, b: u32, op: u32) -> u32 {
        assign << 4 | dst << 8 | a << 11 | b << 14 | op << 17
    }
    fn branch(a: u32, not_zero: bool, offset: i32) -> u32 {
        7 | u32::from(not_zero) << 4 | 0x20 | a << 11 | ((offset as u32) << 14)
    }
    fn w(method: u32, data: u32) -> MethodWrite {
        MethodWrite {
            subchannel: 0,
            method_word: (method / 4) as u16,
            data,
        }
    }
    fn upload(index: u32, code: &[u32]) -> Vec<MethodWrite> {
        let mut writes = vec![w(0, crate::MAXWELL_B), w(0x114, 0x40 * index)];
        writes.extend(code.iter().map(|&c| w(0x118, c)));
        writes.extend([w(0x11c, index), w(0x120, 0x40 * index)]);
        writes
    }
    fn data(out: &[MethodWrite]) -> Vec<(u32, u32)> {
        out.iter()
            .skip(1)
            .map(|w| (w.method_byte_offset(), w.data))
            .collect()
    }
    /// ALU add r0 + r0, moved into r0: no effect.
    const NOP: u32 = MOVE << 4;

    #[test]
    fn sends_parameters_with_method_increment() {
        let mut state = MacroState::default();
        let mut writes = upload(
            0,
            &[
                imm(SET_METHOD, 0, 0, 0x400 | 1 << 12),
                alu(SEND, 0, 1, 0, 0),
                imm(FETCH, 3, 0, 0),
                alu(SEND, 0, 3, 0, 0) | EXIT,
                NOP,
            ],
        );
        writes.extend([w(0x3800, 11), w(0x3804, 22), w(0x2000, 5)]);
        let out = state.expand(&writes).unwrap();
        assert_eq!(data(&out), [(0x1000, 11), (0x1004, 22), (0x2000, 5)]);
        assert_eq!(state.registers[0x401], 22);
    }

    #[test]
    fn branches_loop_and_read_registers() {
        let mut state = MacroState::default();
        let mut writes = upload(
            1,
            &[
                imm(SET_METHOD, 0, 0, 0x500),
                imm(SEND, 1, 1, -1),
                branch(1, true, -1),
                imm(MOVE, 2, 0, 0x480) | 5, // READ register 0x480 into r2
                alu(SEND, 0, 2, 0, 0) | EXIT,
                NOP,
            ],
        );
        writes.extend([w(0x1200, 77), w(0x3808, 3)]);
        let out = state.expand(&writes).unwrap();
        assert_eq!(
            data(&out),
            [
                (0x1200, 77),
                (0x1400, 2),
                (0x1400, 1),
                (0x1400, 0),
                (0x1400, 77)
            ]
        );
    }

    #[test]
    fn branch_with_exit_loops_then_exits_when_not_taken() {
        let mut state = MacroState::default();
        // Z-A pattern: send, decrement, "if r1 != 0 goto -2 (delay slot) else exit".
        let delayed = |offset: i32| 7 | 1 << 4 | EXIT | 1 << 11 | ((offset as u32) << 14);
        let mut writes = upload(
            3,
            &[
                imm(SET_METHOD, 0, 0, 0x700),
                imm(SEND, 1, 1, -1),
                delayed(-1),
                imm(SEND, 0, 0, 9),    // delay slot: runs on every iteration
                imm(SEND, 0, 0, 0x55), // must not run
            ],
        );
        writes.push(w(0x3818, 2));
        let out = state.expand(&writes).unwrap();
        assert_eq!(
            data(&out),
            [(0x1c00, 1), (0x1c00, 9), (0x1c00, 0), (0x1c00, 9)]
        );
    }

    #[test]
    fn alu_carry_and_bitfields() {
        let mut state = MacroState::default();
        let mut writes = upload(
            2,
            &[
                imm(SET_METHOD, 0, 0, 0x600 | 1 << 12),
                imm(FETCH, 2, 0, 0),   // r2 = 0xffff_ffff
                alu(MOVE, 3, 2, 1, 0), // r3 = r2 + r1 (carry)
                alu(SEND, 0, 0, 0, 1), // 0 + 0 + carry = 1
                // r4 = bitfield insert: r1 bits [4..8) into r0 at bit 8
                2 | MOVE << 4 | 4 << 8 | 0 << 11 | 1 << 14 | 4 << 17 | 4 << 22 | 8 << 27,
                alu(SEND, 0, 4, 0, 0),
                alu(SEND, 0, 2, 1, 2) | EXIT, // r2 - r1
                NOP,
            ],
        );
        writes.extend([w(0x3810, 0x35), w(0x3814, 0xffff_ffff)]);
        let out = state.expand(&writes).unwrap();
        assert_eq!(
            data(&out),
            [(0x1800, 1), (0x1804, 0x300), (0x1808, 0xffff_ffca)]
        );
    }

    #[test]
    fn failures_are_explicit() {
        let run = |code: &[u32], call: &[MethodWrite]| {
            let mut state = MacroState::default();
            let mut writes = upload(0, code);
            writes.extend_from_slice(call);
            state.expand(&writes).unwrap_err()
        };
        let exit = [imm(FETCH, 2, 0, 0) | EXIT, NOP];
        assert_eq!(
            run(&exit, &[w(0x3800, 1)]),
            ExecutionError::Macro(MacroError::ParameterUnderflow {
                macro_index: 0,
                pc: 0
            })
        );
        assert_eq!(
            run(&exit, &[w(0x3808, 1)]),
            ExecutionError::Macro(MacroError::UnloadedMacro(1))
        );
        assert_eq!(
            run(&[branch(0, false, 0)], &[w(0x3800, 1)]),
            ExecutionError::Macro(MacroError::StepLimit(0))
        );
        assert_eq!(
            run(&exit, &[w(0x3804, 1)]),
            ExecutionError::Macro(MacroError::ParameterWithoutCall(0x3804))
        );
        assert_eq!(
            run(&exit, &[w(0x124, 3)]),
            ExecutionError::Macro(MacroError::ShadowReplay)
        );
        assert_eq!(
            run(&[imm(MOVE, 0, 0, 0xe00) | 5 | EXIT, NOP], &[w(0x3800, 1)]),
            ExecutionError::Macro(MacroError::ReadOutOfRange {
                pc: 0,
                register: 0xe00
            })
        );
    }

    #[test]
    fn non_threed_subchannels_pass_through() {
        let mut state = MacroState::default();
        let writes = [w(0, crate::MAXWELL_COMPUTE_B), w(0x3800, 1), w(0x118, 2)];
        assert_eq!(state.expand(&writes).unwrap(), writes);
    }
}
