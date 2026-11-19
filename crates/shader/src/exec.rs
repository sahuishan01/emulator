//! Host reference executor for decoded Maxwell programs.
//!
//! Runs one invocation over the [`Cfg`] with scalar f32 semantics so the
//! decoder and the later SPIR-V translation have an independent reference.
//! Forms whose semantics are not modelled (XMAD CSFU/carry/CC, texture fetches, IADD with
//! carry or saturation, condition-code control flow) fail explicitly.

use crate::cfg::{Cfg, Terminator};
use crate::maxwell::{Instr, Listing, MufuOp, Pred, Source, XmadMode, PT, RZ};

/// Inputs an invocation reads from outside the register file.
pub trait Environment {
    /// 32-bit word of c[bank][offset]; None when the buffer is unbound or short.
    fn constant(&self, bank: u8, offset: u32) -> Option<u32>;
    /// Interpolated input attribute at byte address `attr` (IPA).
    fn attribute(&self, attr: u16) -> Option<u32>;
    /// RGBA sample of texture `handle` at (x, y) and explicit level of detail.
    /// None when the texture is unbound or unsupported.
    fn texture_2d(&self, _handle: u16, _x: f32, _y: f32, _lod: f32) -> Option<[f32; 4]> {
        None
    }
}

/// TEXS component selection from memory of the public encoding (unverified):
/// with d1 = RZ one or two components go to d0, d0+1; otherwise three or
/// four components go to d0, d0+1, d1, d1+1.
pub fn texs_components(mask: u8, two_registers: bool) -> Option<&'static [usize]> {
    Some(match (two_registers, mask) {
        (false, 0) => &[0],
        (false, 1) => &[1],
        (false, 2) => &[2],
        (false, 3) => &[3],
        (false, 4) => &[0, 1],
        (false, 5) => &[0, 3],
        (false, 6) => &[1, 3],
        (false, 7) => &[2, 3],
        (true, 0) => &[0, 1, 2],
        (true, 1) => &[0, 1, 3],
        (true, 2) => &[0, 2, 3],
        (true, 3) => &[1, 2, 3],
        (true, 4) => &[0, 1, 2, 3],
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecError {
    Unsupported {
        at: usize,
        instr: Instr,
    },
    Constant {
        at: usize,
        bank: u8,
        offset: u32,
    },
    Attribute {
        at: usize,
        attr: u16,
    },
    BadCompare {
        at: usize,
        cmp: u8,
    },
    BadBoolOp {
        at: usize,
        op: u8,
    },
    BadScale {
        at: usize,
        scale: u8,
    },
    Texture {
        at: usize,
        handle: u16,
    },
    /// The invocation did not exit within the step budget.
    StepBudget,
}

/// Register and predicate state after EXIT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub registers: [u32; 255],
    /// P0..P6 (PT is implicit).
    pub predicates: [bool; 7],
}

impl Default for State {
    fn default() -> Self {
        State {
            registers: [0; 255],
            predicates: [false; 7],
        }
    }
}

impl State {
    fn reg(&self, r: u8) -> u32 {
        if r == RZ {
            0
        } else {
            self.registers[r as usize]
        }
    }
    fn set(&mut self, r: u8, v: u32) {
        if r != RZ {
            self.registers[r as usize] = v;
        }
    }
    fn pred(&self, p: Pred) -> bool {
        let v = if p.index == PT {
            true
        } else {
            self.predicates[p.index as usize]
        };
        v != p.negate
    }
    fn set_pred(&mut self, index: u8, v: bool) {
        if index != PT {
            self.predicates[index as usize] = v;
        }
    }
}

/// Maxwell float compare codes (ordered 1-6, unordered 9-14).
pub fn compare(cmp: u8, a: f32, b: f32) -> Option<bool> {
    let unordered = a.is_nan() || b.is_nan();
    let base = |c: u8| match c {
        1 => a < b,
        2 => a == b,
        3 => a <= b,
        4 => a > b,
        5 => a != b && !unordered,
        6 => a >= b,
        _ => unreachable!(),
    };
    Some(match cmp {
        0 => false,
        1..=6 => base(cmp),
        7 => !unordered,
        8 => unordered,
        9..=14 => unordered || base(cmp - 8),
        15 => true,
        _ => return None,
    })
}

fn bool_op(op: u8, a: bool, b: bool) -> Option<bool> {
    match op {
        0 => Some(a && b),
        1 => Some(a || b),
        2 => Some(a != b),
        _ => None,
    }
}

fn modify(x: f32, abs: bool, neg: bool) -> f32 {
    let x = if abs { x.abs() } else { x };
    if neg {
        -x
    } else {
        x
    }
}

fn saturate(x: f32, sat: bool) -> f32 {
    if sat {
        // NaN saturates to 0.
        if x.is_nan() {
            0.0
        } else {
            x.clamp(0.0, 1.0)
        }
    } else {
        x
    }
}

/// Execute one invocation from the entry block, starting from `state`.
pub fn run(
    listing: &Listing,
    cfg: &Cfg,
    env: &dyn Environment,
    mut state: State,
    max_steps: usize,
) -> Result<State, ExecError> {
    let code = &listing.instructions;
    let mut block = cfg.entry();
    let mut steps = 0usize;
    loop {
        let b = &cfg.blocks[&block];
        for &(at, decoded) in &code[b.instructions.clone()] {
            steps += 1;
            if steps > max_steps {
                return Err(ExecError::StepBudget);
            }
            if !state.pred(decoded.guard) {
                continue;
            }
            step(at, decoded.instr, env, &mut state)?;
        }
        let guard_holds = |g: Pred| state.pred(g);
        block = match b.terminator {
            Terminator::Fallthrough(n) | Terminator::Jump(n) => n,
            Terminator::Branch {
                guard,
                taken,
                not_taken,
            } => {
                if guard_holds(guard) {
                    taken
                } else {
                    not_taken
                }
            }
            Terminator::Exit => return Ok(state),
            Terminator::ExitIf { guard, not_taken } => {
                if guard_holds(guard) {
                    return Ok(state);
                }
                not_taken
            }
        };
    }
}

fn step(at: usize, instr: Instr, env: &dyn Environment, s: &mut State) -> Result<(), ExecError> {
    let operand = |s: &State, src: Source, float: bool| -> Result<u32, ExecError> {
        Ok(match src {
            Source::Reg(r) => s.reg(r),
            Source::Cbuf { bank, offset } => env
                .constant(bank, offset)
                .ok_or(ExecError::Constant { at, bank, offset })?,
            Source::Imm20(i) if float => Source::imm_f32_bits(i),
            Source::Imm20(i) => Source::imm_i32(i) as u32,
        })
    };
    let f = |x: u32| f32::from_bits(x);
    match instr {
        // Control flow is handled by the CFG terminators.
        Instr::Nop
        | Instr::Exit { .. }
        | Instr::Bra { .. }
        | Instr::Ssy { .. }
        | Instr::Sync { .. } => {}
        Instr::Mov { d, b, .. } => {
            let v = operand(s, b, false)?;
            s.set(d, v)
        }
        Instr::Mov32i { d, imm, .. } => s.set(d, imm),
        Instr::Fadd {
            d,
            a,
            b,
            neg_a,
            abs_a,
            neg_b,
            abs_b,
            sat,
        } => {
            let x = modify(f(s.reg(a)), abs_a, neg_a);
            let y = modify(f(operand(s, b, true)?), abs_b, neg_b);
            s.set(d, saturate(x + y, sat).to_bits())
        }
        Instr::Fmul {
            d,
            a,
            b,
            neg_b,
            sat,
            scale,
        } => {
            let k = match scale {
                0 => 1.0,
                1 => 0.5,
                2 => 0.25,
                3 => 0.125,
                4 => 8.0,
                5 => 4.0,
                6 => 2.0,
                _ => return Err(ExecError::BadScale { at, scale }),
            };
            let y = modify(f(operand(s, b, true)?), false, neg_b);
            s.set(d, saturate(f(s.reg(a)) * y * k, sat).to_bits())
        }
        Instr::Fmul32i { d, a, imm, sat } => {
            s.set(d, saturate(f(s.reg(a)) * f(imm), sat).to_bits())
        }
        Instr::Ffma {
            d,
            a,
            b,
            c,
            neg_b,
            neg_c,
            sat,
        } => {
            let y = modify(f(operand(s, b, true)?), false, neg_b);
            let z = modify(f(operand(s, c, true)?), false, neg_c);
            s.set(d, saturate(f(s.reg(a)).mul_add(y, z), sat).to_bits())
        }
        Instr::Fmnmx {
            d,
            a,
            b,
            pred,
            neg_a,
            abs_a,
            neg_b,
            abs_b,
        } => {
            let x = modify(f(s.reg(a)), abs_a, neg_a);
            let y = modify(f(operand(s, b, true)?), abs_b, neg_b);
            let v = if s.pred(pred) { x.min(y) } else { x.max(y) };
            s.set(d, v.to_bits())
        }
        Instr::Fsetp {
            p,
            q,
            a,
            b,
            cmp,
            bop,
            c,
            neg_a,
            abs_a,
            neg_b,
            abs_b,
        } => {
            let x = modify(f(s.reg(a)), abs_a, neg_a);
            let y = modify(f(operand(s, b, true)?), abs_b, neg_b);
            let r = compare(cmp, x, y).ok_or(ExecError::BadCompare { at, cmp })?;
            let cv = s.pred(c);
            let pv = bool_op(bop, r, cv).ok_or(ExecError::BadBoolOp { at, op: bop })?;
            let qv = bool_op(bop, !r, cv).expect("checked");
            s.set_pred(p, pv);
            s.set_pred(q, qv);
        }
        Instr::Psetp {
            p,
            q,
            a,
            b,
            c,
            bop1,
            bop2,
        } => {
            let r =
                bool_op(bop1, s.pred(a), s.pred(b)).ok_or(ExecError::BadBoolOp { at, op: bop1 })?;
            let cv = s.pred(c);
            let pv = bool_op(bop2, r, cv).ok_or(ExecError::BadBoolOp { at, op: bop2 })?;
            let qv = bool_op(bop2, !r, cv).expect("checked");
            s.set_pred(p, pv);
            s.set_pred(q, qv);
        }
        Instr::Mufu {
            d,
            a,
            op,
            neg,
            abs,
            sat,
        } => {
            let x = modify(f(s.reg(a)), abs, neg);
            let v = match op {
                MufuOp::Cos => x.cos(),
                MufuOp::Sin => x.sin(),
                MufuOp::Ex2 => x.exp2(),
                MufuOp::Lg2 => x.log2(),
                MufuOp::Rcp => 1.0 / x,
                MufuOp::Rsq => 1.0 / x.sqrt(),
                MufuOp::Sqrt => x.sqrt(),
                MufuOp::Rcp64h | MufuOp::Rsq64h => {
                    return Err(ExecError::Unsupported { at, instr })
                }
            };
            s.set(d, saturate(v, sat).to_bits())
        }
        Instr::Iadd {
            d,
            a,
            b,
            neg_a,
            neg_b,
            sat: false,
            x: false,
        } => {
            let mut x = s.reg(a);
            let mut y = operand(s, b, false)?;
            if neg_a {
                x = x.wrapping_neg();
            }
            if neg_b {
                y = y.wrapping_neg();
            }
            s.set(d, x.wrapping_add(y))
        }
        Instr::Fcmp { d, a, b, c, cmp } => {
            let z = f(operand(s, c, true)?);
            let r = compare(cmp, z, 0.0).ok_or(ExecError::BadCompare { at, cmp })?;
            let v = if r { s.reg(a) } else { operand(s, b, true)? };
            s.set(d, v)
        }
        Instr::Fset {
            d,
            a,
            b,
            cmp,
            bop,
            c,
            neg_a,
            abs_a,
            neg_b,
            abs_b,
            float_result,
        } => {
            let x = modify(f(s.reg(a)), abs_a, neg_a);
            let y = modify(f(operand(s, b, true)?), abs_b, neg_b);
            let r = compare(cmp, x, y).ok_or(ExecError::BadCompare { at, cmp })?;
            let v = bool_op(bop, r, s.pred(c)).ok_or(ExecError::BadBoolOp { at, op: bop })?;
            let out = match (v, float_result) {
                (false, _) => 0,
                (true, true) => 1.0f32.to_bits(),
                (true, false) => u32::MAX,
            };
            s.set(d, out)
        }
        Instr::Ipa {
            d,
            attr,
            index,
            multiplier,
            mode,
            sample: 0,
            sat,
        } if index == RZ && matches!(mode, 0 | 1) => {
            let v = f(env
                .attribute(attr)
                .ok_or(ExecError::Attribute { at, attr })?);
            let v = if mode == 1 {
                v * f(s.reg(multiplier))
            } else {
                v
            };
            s.set(d, saturate(v, sat).to_bits())
        }
        Instr::Xmad {
            d,
            a,
            b,
            c,
            sign_a,
            sign_b,
            high_a,
            high_b,
            psl,
            mrg,
            x: false,
            cc: false,
            mode,
        } if mode != XmadMode::Csfu => {
            let half = |v: u32, high: bool, signed: bool| -> u32 {
                let h = if high { v >> 16 } else { v & 0xffff };
                if signed {
                    h as u16 as i16 as i32 as u32
                } else {
                    h
                }
            };
            let bv = match b {
                // Immediate forms hold an unsigned 16-bit value.
                Source::Imm20(i) => i & 0xffff,
                _ => operand(s, b, false)?,
            };
            let mut product = half(s.reg(a), high_a, sign_a).wrapping_mul(half(bv, high_b, sign_b));
            if psl {
                product <<= 16;
            }
            let cv = s.reg(c);
            let addend = match mode {
                XmadMode::Full => cv,
                XmadMode::Lo => cv & 0xffff,
                XmadMode::Hi => cv >> 16,
                XmadMode::Bcc => cv.wrapping_add(bv << 16),
                XmadMode::Csfu => unreachable!(),
            };
            let mut v = product.wrapping_add(addend);
            if mrg {
                v = (v & 0xffff) | (bv << 16);
            }
            s.set(d, v)
        }
        // Kind 2: 2D with level of detail zero; coordinates in a, b.
        Instr::Texs {
            d0,
            d1,
            a,
            b,
            handle,
            kind: 2,
            mask,
        } => {
            let comps =
                texs_components(mask, d1 != RZ).ok_or(ExecError::Unsupported { at, instr })?;
            let texel = env
                .texture_2d(handle, f(s.reg(a)), f(s.reg(b)), 0.0)
                .ok_or(ExecError::Texture { at, handle })?;
            let targets = [d0, d0.saturating_add(1), d1, d1.saturating_add(1)];
            if d0 == RZ {
                return Err(ExecError::Unsupported { at, instr });
            }
            for (i, c) in comps.iter().enumerate() {
                s.set(targets[i], texel[*c].to_bits());
            }
        }
        _ => return Err(ExecError::Unsupported { at, instr }),
    }
    Ok(())
}
