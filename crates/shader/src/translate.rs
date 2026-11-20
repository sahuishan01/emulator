//! Maxwell fragment program -> SPIR-V 1.0 (Vulkan) translation.
//!
//! Translates a decoded [`Listing`] + [`Cfg`] with the same semantics as the
//! host reference executor ([`crate::exec`]); every instruction form the
//! executor rejects is rejected here too, and nothing is skipped.
//!
//! Control flow uses a dispatcher loop so any reducible or irreducible CFG
//! the [`Cfg`] builder accepts stays structured:
//!
//! ```text
//! loop { switch (pc) { case block_i: body; pc = next; } }
//! ```
//!
//! EXIT writes the SPH output registers and returns. Registers are Function
//! variables (u32 bit patterns), predicates are bool variables.
//!
//! Host interface (contract with the renderer):
//! - constant bank `n`: uniform block `{ uvec4 data[4096]; }`, set 0 binding `n`;
//! - texture handle `h`: combined 2D float sampler, set 1 binding `h`;
//! - attribute 0x70..0x7c: `FragCoord.xyzw` (W is 1/w_clip in both APIs);
//! - generic attribute 0x80 + 16n + 4c: smooth vec4 input, Location `n`.
//!   The hardware interpolates attr/w linearly, so IPA PASS yields
//!   `input * FragCoord.w` and IPA MUL multiplies that by the register;
//!   perspective-correct shaders (`IPA.MUL attr, rcp(a[0x7c])`) thus give
//!   exactly the smooth Vulkan input. Flat/noperspective SPH modes are not
//!   read yet (all generics are smooth);
//! - colour target `t`: vec4 output Location `t`; disabled components are 0;
//! - depth output: `FragDepth` (DepthReplacing). Sample mask output fails.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::cfg::{Cfg, Terminator};
use crate::maxwell::{Instr, Listing, MufuOp, Pred, Source, XmadMode, PT, RZ};
use crate::sph::Header;

/// Largest module accepted (words).
pub const MAX_WORDS: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranslateError {
    NotPixel,
    /// SPH output layout unreadable or uses a sample mask output.
    Outputs,
    Unsupported {
        at: usize,
        instr: Instr,
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
    /// Constant offset unaligned or past the 64 KiB bank.
    Constant {
        at: usize,
        bank: u8,
        offset: u32,
    },
    /// IPA attribute outside FragCoord / generic inputs.
    Attribute {
        at: usize,
        attr: u16,
    },
    TooLarge,
}

// Opcodes.
const OP_EXT_INST_IMPORT: u32 = 11;
const OP_EXT_INST: u32 = 12;
const OP_MEMORY_MODEL: u32 = 14;
const OP_ENTRY_POINT: u32 = 15;
const OP_EXECUTION_MODE: u32 = 16;
const OP_CAPABILITY: u32 = 17;
const OP_TYPE_VOID: u32 = 19;
const OP_TYPE_BOOL: u32 = 20;
const OP_TYPE_INT: u32 = 21;
const OP_TYPE_FLOAT: u32 = 22;
const OP_TYPE_VECTOR: u32 = 23;
const OP_TYPE_IMAGE: u32 = 25;
const OP_TYPE_SAMPLED_IMAGE: u32 = 27;
const OP_TYPE_ARRAY: u32 = 28;
const OP_TYPE_STRUCT: u32 = 30;
const OP_TYPE_POINTER: u32 = 32;
const OP_TYPE_FUNCTION: u32 = 33;
const OP_CONSTANT_TRUE: u32 = 41;
const OP_CONSTANT_FALSE: u32 = 42;
const OP_CONSTANT: u32 = 43;
const OP_FUNCTION: u32 = 54;
const OP_FUNCTION_END: u32 = 56;
const OP_VARIABLE: u32 = 59;
const OP_LOAD: u32 = 61;
const OP_STORE: u32 = 62;
const OP_ACCESS_CHAIN: u32 = 65;
const OP_DECORATE: u32 = 71;
const OP_MEMBER_DECORATE: u32 = 72;
const OP_COMPOSITE_CONSTRUCT: u32 = 80;
const OP_COMPOSITE_EXTRACT: u32 = 81;
const OP_IMAGE_SAMPLE_EXPLICIT_LOD: u32 = 88;
const OP_IMAGE: u32 = 100;
const OP_IMAGE_QUERY_SIZE_LOD: u32 = 103;
const OP_CONVERT_S_TO_F: u32 = 111;
const OP_BITCAST: u32 = 124;
const OP_S_NEGATE: u32 = 126;
const OP_F_NEGATE: u32 = 127;
const OP_I_ADD: u32 = 128;
const OP_F_ADD: u32 = 129;
const OP_I_MUL: u32 = 132;
const OP_F_MUL: u32 = 133;
const OP_F_DIV: u32 = 136;
const OP_ORDERED: u32 = 162;
const OP_UNORDERED: u32 = 163;
const OP_LOGICAL_NOT_EQUAL: u32 = 165;
const OP_LOGICAL_OR: u32 = 166;
const OP_LOGICAL_AND: u32 = 167;
const OP_LOGICAL_NOT: u32 = 168;
const OP_SELECT: u32 = 169;
const OP_SHIFT_RIGHT_LOGICAL: u32 = 194;
const OP_SHIFT_LEFT_LOGICAL: u32 = 196;
const OP_BITWISE_OR: u32 = 197;
const OP_BITWISE_AND: u32 = 199;
const OP_BIT_FIELD_S_EXTRACT: u32 = 202;
const OP_BIT_FIELD_U_EXTRACT: u32 = 203;
const OP_LOOP_MERGE: u32 = 246;
const OP_SELECTION_MERGE: u32 = 247;
const OP_LABEL: u32 = 248;
const OP_BRANCH: u32 = 249;
const OP_BRANCH_CONDITIONAL: u32 = 250;
const OP_SWITCH: u32 = 251;
const OP_RETURN: u32 = 253;
const OP_UNREACHABLE: u32 = 255;

// GLSL.std.450.
const GLSL_FABS: u32 = 4;
const GLSL_SIN: u32 = 13;
const GLSL_COS: u32 = 14;
const GLSL_EXP2: u32 = 29;
const GLSL_LOG2: u32 = 30;
const GLSL_SQRT: u32 = 31;
const GLSL_INVERSE_SQRT: u32 = 32;
const GLSL_FMA: u32 = 50;
const GLSL_NMIN: u32 = 79;
const GLSL_NMAX: u32 = 80;

const SC_UNIFORM_CONSTANT: u32 = 0;
const SC_INPUT: u32 = 1;
const SC_UNIFORM: u32 = 2;
const SC_OUTPUT: u32 = 3;
const SC_FUNCTION: u32 = 7;

const DEC_BLOCK: u32 = 2;
const DEC_ARRAY_STRIDE: u32 = 6;
const DEC_BUILTIN: u32 = 11;
const DEC_LOCATION: u32 = 30;
const DEC_BINDING: u32 = 33;
const DEC_DESCRIPTOR_SET: u32 = 34;
const DEC_OFFSET: u32 = 35;
const BUILTIN_FRAG_COORD: u32 = 15;
const BUILTIN_FRAG_DEPTH: u32 = 22;

/// uvec4 entries per constant bank (64 KiB).
const CBUF_VEC4S: u32 = 4096;

fn push(out: &mut Vec<u32>, op: u32, args: &[u32]) {
    out.push(((args.len() as u32 + 1) << 16) | op);
    out.extend_from_slice(args);
}

fn string_words(s: &str) -> Vec<u32> {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().expect("4 bytes")))
        .collect()
}

struct Types {
    void: u32,
    bool_: u32,
    uint: u32,
    int: u32,
    float: u32,
    v2f: u32,
    v2i: u32,
    v4f: u32,
    ptr_fn_uint: u32,
    ptr_fn_bool: u32,
}

struct Module {
    next: u32,
    glsl: u32,
    decorations: Vec<u32>,
    globals: Vec<u32>,
    /// Function-scope OpVariables (spliced after the entry label).
    locals: Vec<u32>,
    body: Vec<u32>,
    interface: Vec<u32>,
    capabilities: BTreeSet<u32>,
    depth_replacing: bool,
    t: Types,
    uint_consts: HashMap<u32, u32>,
    float_consts: HashMap<u32, u32>,
    int_consts: HashMap<i32, u32>,
    true_: u32,
    false_: u32,
    registers: BTreeMap<u8, u32>,
    predicates: BTreeMap<u8, u32>,
    banks: BTreeMap<u8, u32>,
    ptr_uniform_uint: Option<u32>,
    generics: BTreeMap<u16, u32>,
    frag_coord: Option<u32>,
    ptr_input_v4f: Option<u32>,
    textures: BTreeMap<u16, u32>,
    sampled_image: Option<u32>,
}

impl Module {
    fn new() -> Module {
        let mut m = Module {
            next: 1,
            glsl: 0,
            decorations: vec![],
            globals: vec![],
            locals: vec![],
            body: vec![],
            interface: vec![],
            capabilities: BTreeSet::from([1]),
            depth_replacing: false,
            t: Types {
                void: 0,
                bool_: 0,
                uint: 0,
                int: 0,
                float: 0,
                v2f: 0,
                v2i: 0,
                v4f: 0,
                ptr_fn_uint: 0,
                ptr_fn_bool: 0,
            },
            uint_consts: HashMap::new(),
            float_consts: HashMap::new(),
            int_consts: HashMap::new(),
            true_: 0,
            false_: 0,
            registers: BTreeMap::new(),
            predicates: BTreeMap::new(),
            banks: BTreeMap::new(),
            ptr_uniform_uint: None,
            generics: BTreeMap::new(),
            frag_coord: None,
            ptr_input_v4f: None,
            textures: BTreeMap::new(),
            sampled_image: None,
        };
        m.glsl = m.id();
        m.t.void = m.global(OP_TYPE_VOID, &[]);
        m.t.bool_ = m.global(OP_TYPE_BOOL, &[]);
        m.t.uint = m.global(OP_TYPE_INT, &[32, 0]);
        m.t.int = m.global(OP_TYPE_INT, &[32, 1]);
        m.t.float = m.global(OP_TYPE_FLOAT, &[32]);
        m.t.v2f = m.global(OP_TYPE_VECTOR, &[m.t.float, 2]);
        m.t.v2i = m.global(OP_TYPE_VECTOR, &[m.t.int, 2]);
        m.t.v4f = m.global(OP_TYPE_VECTOR, &[m.t.float, 4]);
        m.t.ptr_fn_uint = m.global(OP_TYPE_POINTER, &[SC_FUNCTION, m.t.uint]);
        m.t.ptr_fn_bool = m.global(OP_TYPE_POINTER, &[SC_FUNCTION, m.t.bool_]);
        m.true_ = m.global(OP_CONSTANT_TRUE, &[m.t.bool_]);
        m.false_ = m.global(OP_CONSTANT_FALSE, &[m.t.bool_]);
        m
    }
    fn id(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }
    /// Global declaration whose result id is the first operand (types).
    fn global(&mut self, op: u32, args: &[u32]) -> u32 {
        let id = self.id();
        let head = if matches!(op, OP_CONSTANT_TRUE | OP_CONSTANT_FALSE | OP_CONSTANT) {
            // Constants: result type first, then id.
            let mut v = vec![args[0], id];
            v.extend_from_slice(&args[1..]);
            v
        } else {
            let mut v = vec![id];
            v.extend_from_slice(args);
            v
        };
        push(&mut self.globals, op, &head);
        id
    }
    fn global_var(&mut self, ptr: u32, class: u32) -> u32 {
        let id = self.id();
        push(&mut self.globals, OP_VARIABLE, &[ptr, id, class]);
        id
    }
    fn decorate(&mut self, target: u32, args: &[u32]) {
        let mut v = vec![target];
        v.extend_from_slice(args);
        push(&mut self.decorations, OP_DECORATE, &v);
    }
    fn uconst(&mut self, v: u32) -> u32 {
        if let Some(&id) = self.uint_consts.get(&v) {
            return id;
        }
        let id = self.global(OP_CONSTANT, &[self.t.uint, v]);
        self.uint_consts.insert(v, id);
        id
    }
    fn iconst(&mut self, v: i32) -> u32 {
        if let Some(&id) = self.int_consts.get(&v) {
            return id;
        }
        let id = self.global(OP_CONSTANT, &[self.t.int, v as u32]);
        self.int_consts.insert(v, id);
        id
    }
    fn fconst(&mut self, v: f32) -> u32 {
        let bits = v.to_bits();
        if let Some(&id) = self.float_consts.get(&bits) {
            return id;
        }
        let id = self.global(OP_CONSTANT, &[self.t.float, bits]);
        self.float_consts.insert(bits, id);
        id
    }
    /// Body instruction with a result.
    fn op(&mut self, op: u32, ty: u32, args: &[u32]) -> u32 {
        let id = self.id();
        let mut v = vec![ty, id];
        v.extend_from_slice(args);
        push(&mut self.body, op, &v);
        id
    }
    /// Body instruction without a result.
    fn stmt(&mut self, op: u32, args: &[u32]) {
        push(&mut self.body, op, args);
    }
    fn ext(&mut self, ty: u32, inst: u32, args: &[u32]) -> u32 {
        let mut v = vec![self.glsl, inst];
        v.extend_from_slice(args);
        self.op(OP_EXT_INST, ty, &v)
    }
    fn label(&mut self, id: u32) {
        self.stmt(OP_LABEL, &[id]);
    }

    fn reg_var(&mut self, r: u8) -> u32 {
        if let Some(&v) = self.registers.get(&r) {
            return v;
        }
        let id = self.id();
        let zero = self.uconst(0);
        push(
            &mut self.locals,
            OP_VARIABLE,
            &[self.t.ptr_fn_uint, id, SC_FUNCTION, zero],
        );
        self.registers.insert(r, id);
        id
    }
    fn pred_var(&mut self, p: u8) -> u32 {
        if let Some(&v) = self.predicates.get(&p) {
            return v;
        }
        let id = self.id();
        let f = self.false_;
        push(
            &mut self.locals,
            OP_VARIABLE,
            &[self.t.ptr_fn_bool, id, SC_FUNCTION, f],
        );
        self.predicates.insert(p, id);
        id
    }
    fn reg(&mut self, r: u8) -> u32 {
        if r == RZ {
            return self.uconst(0);
        }
        let var = self.reg_var(r);
        let ty = self.t.uint;
        self.op(OP_LOAD, ty, &[var])
    }
    fn set(&mut self, r: u8, value: u32) {
        if r != RZ {
            let var = self.reg_var(r);
            self.stmt(OP_STORE, &[var, value]);
        }
    }
    fn freg(&mut self, r: u8) -> u32 {
        let u = self.reg(r);
        self.as_float(u)
    }
    fn fset(&mut self, r: u8, value: u32) {
        let u = self.as_bits(value);
        self.set(r, u);
    }
    fn as_float(&mut self, u: u32) -> u32 {
        let ty = self.t.float;
        self.op(OP_BITCAST, ty, &[u])
    }
    fn as_bits(&mut self, f: u32) -> u32 {
        let ty = self.t.uint;
        self.op(OP_BITCAST, ty, &[f])
    }
    fn pred(&mut self, p: Pred) -> u32 {
        let v = if p.index == PT {
            self.true_
        } else {
            let var = self.pred_var(p.index);
            let ty = self.t.bool_;
            self.op(OP_LOAD, ty, &[var])
        };
        if p.negate {
            let ty = self.t.bool_;
            self.op(OP_LOGICAL_NOT, ty, &[v])
        } else {
            v
        }
    }
    fn set_pred(&mut self, p: u8, v: u32) {
        if p != PT {
            let var = self.pred_var(p);
            self.stmt(OP_STORE, &[var, v]);
        }
    }

    fn constant(&mut self, at: usize, bank: u8, offset: u32) -> Result<u32, TranslateError> {
        if offset % 4 != 0 || offset >= CBUF_VEC4S * 16 {
            return Err(TranslateError::Constant { at, bank, offset });
        }
        let var = match self.banks.get(&bank) {
            Some(&v) => v,
            None => {
                let uint = self.t.uint;
                let v4u = self.global(OP_TYPE_VECTOR, &[uint, 4]);
                let len = self.uconst(CBUF_VEC4S);
                let array = self.global(OP_TYPE_ARRAY, &[v4u, len]);
                self.decorate(array, &[DEC_ARRAY_STRIDE, 16]);
                let block = self.global(OP_TYPE_STRUCT, &[array]);
                self.decorate(block, &[DEC_BLOCK]);
                push(
                    &mut self.decorations,
                    OP_MEMBER_DECORATE,
                    &[block, 0, DEC_OFFSET, 0],
                );
                let ptr = self.global(OP_TYPE_POINTER, &[SC_UNIFORM, block]);
                let var = self.global_var(ptr, SC_UNIFORM);
                self.decorate(var, &[DEC_DESCRIPTOR_SET, 0]);
                self.decorate(var, &[DEC_BINDING, u32::from(bank)]);
                self.banks.insert(bank, var);
                var
            }
        };
        let ptr = match self.ptr_uniform_uint {
            Some(p) => p,
            None => {
                let uint = self.t.uint;
                let p = self.global(OP_TYPE_POINTER, &[SC_UNIFORM, uint]);
                self.ptr_uniform_uint = Some(p);
                p
            }
        };
        let zero = self.uconst(0);
        let vec = self.uconst(offset / 16);
        let comp = self.uconst((offset / 4) % 4);
        let chain = self.op(OP_ACCESS_CHAIN, ptr, &[var, zero, vec, comp]);
        let ty = self.t.uint;
        Ok(self.op(OP_LOAD, ty, &[chain]))
    }

    fn operand(&mut self, at: usize, src: Source, float: bool) -> Result<u32, TranslateError> {
        Ok(match src {
            Source::Reg(r) => self.reg(r),
            Source::Cbuf { bank, offset } => self.constant(at, bank, offset)?,
            Source::Imm20(i) if float => self.uconst(Source::imm_f32_bits(i)),
            Source::Imm20(i) => self.uconst(Source::imm_i32(i) as u32),
        })
    }
    fn foperand(&mut self, at: usize, src: Source) -> Result<u32, TranslateError> {
        let u = self.operand(at, src, true)?;
        Ok(self.as_float(u))
    }

    fn modify(&mut self, x: u32, abs: bool, neg: bool) -> u32 {
        let ty = self.t.float;
        let x = if abs {
            self.ext(ty, GLSL_FABS, &[x])
        } else {
            x
        };
        if neg {
            self.op(OP_F_NEGATE, ty, &[x])
        } else {
            x
        }
    }
    /// NaN saturates to 0 (NMax drops the NaN).
    fn saturate(&mut self, x: u32, sat: bool) -> u32 {
        if !sat {
            return x;
        }
        let ty = self.t.float;
        let zero = self.fconst(0.0);
        let one = self.fconst(1.0);
        let lo = self.ext(ty, GLSL_NMAX, &[x, zero]);
        self.ext(ty, GLSL_NMIN, &[lo, one])
    }
    fn compare(&mut self, at: usize, cmp: u8, a: u32, b: u32) -> Result<u32, TranslateError> {
        let op = match cmp {
            0 => return Ok(self.false_),
            15 => return Ok(self.true_),
            1 => 184,
            2 => 180,
            3 => 188,
            4 => 186,
            5 => 182,
            6 => 190,
            7 => OP_ORDERED,
            8 => OP_UNORDERED,
            9 => 185,
            10 => 181,
            11 => 189,
            12 => 187,
            13 => 183,
            14 => 191,
            _ => return Err(TranslateError::BadCompare { at, cmp }),
        };
        let ty = self.t.bool_;
        Ok(self.op(op, ty, &[a, b]))
    }
    fn bool_op(&mut self, at: usize, op: u8, a: u32, b: u32) -> Result<u32, TranslateError> {
        let code = match op {
            0 => OP_LOGICAL_AND,
            1 => OP_LOGICAL_OR,
            2 => OP_LOGICAL_NOT_EQUAL,
            _ => return Err(TranslateError::BadBoolOp { at, op }),
        };
        let ty = self.t.bool_;
        Ok(self.op(code, ty, &[a, b]))
    }

    fn attribute(&mut self, at: usize, attr: u16) -> Result<u32, TranslateError> {
        let ptr_in = match self.ptr_input_v4f {
            Some(p) => p,
            None => {
                let v4f = self.t.v4f;
                let p = self.global(OP_TYPE_POINTER, &[SC_INPUT, v4f]);
                self.ptr_input_v4f = Some(p);
                p
            }
        };
        let fc = self.frag_coord_w(ptr_in);
        let ty = self.t.float;
        match attr {
            0x70..=0x7c if attr % 4 == 0 => {
                let var = self.frag_coord.expect("created");
                let v = self.op(OP_LOAD, self.t.v4f, &[var]);
                Ok(self.op(OP_COMPOSITE_EXTRACT, ty, &[v, u32::from((attr - 0x70) / 4)]))
            }
            0x80..=0x27c if attr % 4 == 0 => {
                let location = (attr - 0x80) / 16;
                let var = match self.generics.get(&location) {
                    Some(&v) => v,
                    None => {
                        let v = self.global_var(ptr_in, SC_INPUT);
                        self.decorate(v, &[DEC_LOCATION, u32::from(location)]);
                        self.interface.push(v);
                        self.generics.insert(location, v);
                        v
                    }
                };
                let v = self.op(OP_LOAD, self.t.v4f, &[var]);
                let c = self.op(OP_COMPOSITE_EXTRACT, ty, &[v, u32::from((attr % 16) / 4)]);
                Ok(self.op(OP_F_MUL, ty, &[c, fc]))
            }
            _ => Err(TranslateError::Attribute { at, attr }),
        }
    }
    /// FragCoord.w (declares FragCoord on first use).
    fn frag_coord_w(&mut self, ptr_in: u32) -> u32 {
        let var = match self.frag_coord {
            Some(v) => v,
            None => {
                let v = self.global_var(ptr_in, SC_INPUT);
                self.decorate(v, &[DEC_BUILTIN, BUILTIN_FRAG_COORD]);
                self.interface.push(v);
                self.frag_coord = Some(v);
                v
            }
        };
        let v = self.op(OP_LOAD, self.t.v4f, &[var]);
        let ty = self.t.float;
        self.op(OP_COMPOSITE_EXTRACT, ty, &[v, 3])
    }

    fn texture(&mut self, handle: u16) -> u32 {
        let sampled = match self.sampled_image {
            Some(s) => s,
            None => {
                let float = self.t.float;
                // 2D, not depth, not arrayed, single-sampled, sampled, Unknown format.
                let image = self.global(OP_TYPE_IMAGE, &[float, 1, 0, 0, 0, 1, 0]);
                let s = self.global(OP_TYPE_SAMPLED_IMAGE, &[image]);
                self.sampled_image = Some(s);
                s
            }
        };
        let var = match self.textures.get(&handle) {
            Some(&v) => v,
            None => {
                let ptr = self.global(OP_TYPE_POINTER, &[SC_UNIFORM_CONSTANT, sampled]);
                let v = self.global_var(ptr, SC_UNIFORM_CONSTANT);
                self.decorate(v, &[DEC_DESCRIPTOR_SET, 1]);
                self.decorate(v, &[DEC_BINDING, u32::from(handle)]);
                self.textures.insert(handle, v);
                v
            }
        };
        self.op(OP_LOAD, sampled, &[var])
    }
    /// RGBA sample at level 0; a dynamic texel offset is applied to the
    /// coordinates (Vulkan forbids a non-constant Offset outside gathers).
    fn sample_lz(&mut self, handle: u16, x: u32, y: u32, offset: Option<u32>) -> u32 {
        let image = self.texture(handle);
        let v2f = self.t.v2f;
        let mut coord = self.op(OP_COMPOSITE_CONSTRUCT, v2f, &[x, y]);
        if let Some(packed) = offset {
            self.capabilities.insert(50); // ImageQuery
            let uint = self.t.uint;
            let int = self.t.int;
            let four = self.uconst(4);
            let zero = self.uconst(0);
            let ox = self.op(OP_BIT_FIELD_S_EXTRACT, uint, &[packed, zero, four]);
            let oy = self.op(OP_BIT_FIELD_S_EXTRACT, uint, &[packed, four, four]);
            let ox = self.op(OP_BITCAST, int, &[ox]);
            let oy = self.op(OP_BITCAST, int, &[oy]);
            let v2i = self.t.v2i;
            let off = self.op(OP_COMPOSITE_CONSTRUCT, v2i, &[ox, oy]);
            let off = self.op(OP_CONVERT_S_TO_F, v2f, &[off]);
            let ty = self.global_image_type();
            let raw = self.op(OP_IMAGE, ty, &[image]);
            let lod = self.iconst(0);
            let size = self.op(OP_IMAGE_QUERY_SIZE_LOD, v2i, &[raw, lod]);
            let size = self.op(OP_CONVERT_S_TO_F, v2f, &[size]);
            let shift = self.op(OP_F_DIV, v2f, &[off, size]);
            coord = self.op(OP_F_ADD, v2f, &[coord, shift]);
        }
        let lod = self.fconst(0.0);
        let v4f = self.t.v4f;
        self.op(OP_IMAGE_SAMPLE_EXPLICIT_LOD, v4f, &[image, coord, 0x2, lod])
    }
    fn global_image_type(&mut self) -> u32 {
        // The image type is the operand of the sampled image type.
        let sampled = self.sampled_image.expect("declared");
        // Find it: OpTypeSampledImage result `sampled` takes the image id.
        let mut i = 0;
        while i < self.globals.len() {
            let words = (self.globals[i] >> 16) as usize;
            if self.globals[i] & 0xffff == OP_TYPE_SAMPLED_IMAGE && self.globals[i + 1] == sampled {
                return self.globals[i + 2];
            }
            i += words.max(1);
        }
        unreachable!("sampled image type declared")
    }
}

/// Translate a fragment program. The SPH supplies the output layout.
pub fn translate_fragment(
    listing: &Listing,
    cfg: &Cfg,
    sph: &Header,
) -> Result<Vec<u32>, TranslateError> {
    let outputs = sph.fragment_outputs().ok_or(TranslateError::NotPixel)?;
    if outputs.sample_mask.is_some() {
        return Err(TranslateError::Outputs);
    }
    let mut m = Module::new();
    let code = &listing.instructions;

    let blocks: Vec<usize> = cfg.blocks.keys().copied().collect();
    let index: BTreeMap<usize, u32> = blocks
        .iter()
        .enumerate()
        .map(|(i, s)| (*s, i as u32))
        .collect();

    // Output variables.
    let ptr_out_v4f = m.global(OP_TYPE_POINTER, &[SC_OUTPUT, m.t.v4f]);
    let mut targets: BTreeMap<u8, u32> = BTreeMap::new();
    for c in &outputs.colors {
        if let std::collections::btree_map::Entry::Vacant(e) = targets.entry(c.target) {
            let v = m.global_var(ptr_out_v4f, SC_OUTPUT);
            m.decorate(v, &[DEC_LOCATION, u32::from(c.target)]);
            m.interface.push(v);
            e.insert(v);
        }
    }
    let depth = match outputs.depth {
        Some(r) => {
            let float = m.t.float;
            let ptr = m.global(OP_TYPE_POINTER, &[SC_OUTPUT, float]);
            let v = m.global_var(ptr, SC_OUTPUT);
            m.decorate(v, &[DEC_BUILTIN, BUILTIN_FRAG_DEPTH]);
            m.interface.push(v);
            m.depth_replacing = true;
            Some((v, r))
        }
        None => None,
    };

    let fn_type = m.global(OP_TYPE_FUNCTION, &[m.t.void]);
    let main = m.id();
    let entry = m.id();
    let header = m.id();
    let dispatch = m.id();
    let sw_merge = m.id();
    let cont = m.id();
    let merge = m.id();
    let labels: Vec<u32> = blocks.iter().map(|_| m.id()).collect();
    let pc = m.id();

    // Entry block body (locals are spliced in front later).
    let entry_value = m.uconst(index[&cfg.entry()]);
    let mut entry_tail = Vec::new();
    push(&mut entry_tail, OP_STORE, &[pc, entry_value]);
    push(&mut entry_tail, OP_BRANCH, &[header]);

    // Loop header and dispatcher.
    m.label(header);
    m.stmt(OP_LOOP_MERGE, &[merge, cont, 0]);
    m.stmt(OP_BRANCH, &[dispatch]);
    m.label(dispatch);
    let pc_value = m.op(OP_LOAD, m.t.uint, &[pc]);
    m.stmt(OP_SELECTION_MERGE, &[sw_merge, 0]);
    let mut switch = vec![pc_value, sw_merge];
    for (i, l) in labels.iter().enumerate() {
        switch.push(i as u32);
        switch.push(*l);
    }
    m.stmt(OP_SWITCH, &switch);

    let write_outputs = |m: &mut Module| {
        for (&target, &var) in &targets {
            let mut comps = [0u32; 4];
            for (i, slot) in comps.iter_mut().enumerate() {
                *slot = match outputs
                    .colors
                    .iter()
                    .find(|c| c.target == target && c.component as usize == i)
                {
                    Some(c) => m.freg(c.register),
                    None => m.fconst(0.0),
                };
            }
            let v4f = m.t.v4f;
            let v = m.op(OP_COMPOSITE_CONSTRUCT, v4f, &comps);
            m.stmt(OP_STORE, &[var, v]);
        }
        if let Some((var, r)) = depth {
            let v = m.freg(r);
            m.stmt(OP_STORE, &[var, v]);
        }
        m.stmt(OP_RETURN, &[]);
    };

    for (i, start) in blocks.iter().enumerate() {
        let block = &cfg.blocks[start];
        m.label(labels[i]);
        for &(at, decoded) in &code[block.instructions.clone()] {
            if is_control(&decoded.instr) {
                continue;
            }
            if decoded.guard.is_always() {
                instruction(&mut m, at, decoded.instr)?;
            } else {
                let cond = m.pred(decoded.guard);
                let then = m.id();
                let after = m.id();
                m.stmt(OP_SELECTION_MERGE, &[after, 0]);
                m.stmt(OP_BRANCH_CONDITIONAL, &[cond, then, after]);
                m.label(then);
                instruction(&mut m, at, decoded.instr)?;
                m.stmt(OP_BRANCH, &[after]);
                m.label(after);
            }
        }
        match block.terminator {
            Terminator::Fallthrough(n) | Terminator::Jump(n) => {
                let v = m.uconst(index[&n]);
                m.stmt(OP_STORE, &[pc, v]);
                m.stmt(OP_BRANCH, &[sw_merge]);
            }
            Terminator::Branch {
                guard,
                taken,
                not_taken,
            } => {
                let cond = m.pred(guard);
                let t = m.uconst(index[&taken]);
                let n = m.uconst(index[&not_taken]);
                let uint = m.t.uint;
                let v = m.op(OP_SELECT, uint, &[cond, t, n]);
                m.stmt(OP_STORE, &[pc, v]);
                m.stmt(OP_BRANCH, &[sw_merge]);
            }
            Terminator::Exit => write_outputs(&mut m),
            Terminator::ExitIf { guard, not_taken } => {
                let cond = m.pred(guard);
                let exit = m.id();
                let stay = m.id();
                m.stmt(OP_SELECTION_MERGE, &[stay, 0]);
                m.stmt(OP_BRANCH_CONDITIONAL, &[cond, exit, stay]);
                m.label(exit);
                write_outputs(&mut m);
                m.label(stay);
                let v = m.uconst(index[&not_taken]);
                m.stmt(OP_STORE, &[pc, v]);
                m.stmt(OP_BRANCH, &[sw_merge]);
            }
        }
    }
    m.label(sw_merge);
    m.stmt(OP_BRANCH, &[cont]);
    m.label(cont);
    m.stmt(OP_BRANCH, &[header]);
    m.label(merge);
    m.stmt(OP_UNREACHABLE, &[]);

    // pc variable.
    let ptr_fn_uint = m.t.ptr_fn_uint;
    push(&mut m.locals, OP_VARIABLE, &[ptr_fn_uint, pc, SC_FUNCTION]);

    // Assemble.
    let mut out = vec![0x0723_0203, 0x0001_0000, 0, m.next, 0];
    for cap in &m.capabilities {
        push(&mut out, OP_CAPABILITY, &[*cap]);
    }
    let mut import = vec![m.glsl];
    import.extend(string_words("GLSL.std.450"));
    push(&mut out, OP_EXT_INST_IMPORT, &import);
    push(&mut out, OP_MEMORY_MODEL, &[0, 1]);
    let mut ep = vec![4, main];
    ep.extend(string_words("main"));
    ep.extend(&m.interface);
    push(&mut out, OP_ENTRY_POINT, &ep);
    push(&mut out, OP_EXECUTION_MODE, &[main, 7]); // OriginUpperLeft
    if m.depth_replacing {
        push(&mut out, OP_EXECUTION_MODE, &[main, 12]);
    }
    out.extend(&m.decorations);
    out.extend(&m.globals);
    push(&mut out, OP_FUNCTION, &[m.t.void, main, 0, fn_type]);
    push(&mut out, OP_LABEL, &[entry]);
    out.extend(&m.locals);
    out.extend(&entry_tail);
    out.extend(&m.body);
    push(&mut out, OP_FUNCTION_END, &[]);
    out[3] = m.next;
    if out.len() > MAX_WORDS {
        return Err(TranslateError::TooLarge);
    }
    Ok(out)
}

fn is_control(instr: &Instr) -> bool {
    matches!(
        instr,
        Instr::Nop
            | Instr::Exit { .. }
            | Instr::Bra { .. }
            | Instr::Ssy { .. }
            | Instr::Sync { .. }
    )
}

/// One non-control instruction; mirrors `exec::step`.
fn instruction(m: &mut Module, at: usize, instr: Instr) -> Result<(), TranslateError> {
    let float = m.t.float;
    let uint = m.t.uint;
    let unsupported = TranslateError::Unsupported { at, instr };
    match instr {
        Instr::Mov { d, b, .. } => {
            let v = m.operand(at, b, false)?;
            m.set(d, v)
        }
        Instr::Mov32i { d, imm, .. } => {
            let v = m.uconst(imm);
            m.set(d, v)
        }
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
            let x = m.freg(a);
            let x = m.modify(x, abs_a, neg_a);
            let y = m.foperand(at, b)?;
            let y = m.modify(y, abs_b, neg_b);
            let v = m.op(OP_F_ADD, float, &[x, y]);
            let v = m.saturate(v, sat);
            m.fset(d, v)
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
                _ => return Err(TranslateError::BadScale { at, scale }),
            };
            let x = m.freg(a);
            let y = m.foperand(at, b)?;
            let y = m.modify(y, false, neg_b);
            let mut v = m.op(OP_F_MUL, float, &[x, y]);
            if scale != 0 {
                let kc = m.fconst(k);
                v = m.op(OP_F_MUL, float, &[v, kc]);
            }
            let v = m.saturate(v, sat);
            m.fset(d, v)
        }
        Instr::Fmul32i { d, a, imm, sat } => {
            let x = m.freg(a);
            let y = m.fconst(f32::from_bits(imm));
            let v = m.op(OP_F_MUL, float, &[x, y]);
            let v = m.saturate(v, sat);
            m.fset(d, v)
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
            let x = m.freg(a);
            let y = m.foperand(at, b)?;
            let y = m.modify(y, false, neg_b);
            let z = m.foperand(at, c)?;
            let z = m.modify(z, false, neg_c);
            let v = m.ext(float, GLSL_FMA, &[x, y, z]);
            let v = m.saturate(v, sat);
            m.fset(d, v)
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
            let x = m.freg(a);
            let x = m.modify(x, abs_a, neg_a);
            let y = m.foperand(at, b)?;
            let y = m.modify(y, abs_b, neg_b);
            let lo = m.ext(float, GLSL_NMIN, &[x, y]);
            let hi = m.ext(float, GLSL_NMAX, &[x, y]);
            let p = m.pred(pred);
            let v = m.op(OP_SELECT, float, &[p, lo, hi]);
            m.fset(d, v)
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
            let x = m.freg(a);
            let x = m.modify(x, abs_a, neg_a);
            let y = m.foperand(at, b)?;
            let y = m.modify(y, abs_b, neg_b);
            let r = m.compare(at, cmp, x, y)?;
            let cv = m.pred(c);
            let pv = m.bool_op(at, bop, r, cv)?;
            let bool_ = m.t.bool_;
            let nr = m.op(OP_LOGICAL_NOT, bool_, &[r]);
            let qv = m.bool_op(at, bop, nr, cv)?;
            m.set_pred(p, pv);
            m.set_pred(q, qv);
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
            let av = m.pred(a);
            let bv = m.pred(b);
            let r = m.bool_op(at, bop1, av, bv)?;
            let cv = m.pred(c);
            let pv = m.bool_op(at, bop2, r, cv)?;
            let bool_ = m.t.bool_;
            let nr = m.op(OP_LOGICAL_NOT, bool_, &[r]);
            let qv = m.bool_op(at, bop2, nr, cv)?;
            m.set_pred(p, pv);
            m.set_pred(q, qv);
        }
        Instr::Mufu {
            d,
            a,
            op,
            neg,
            abs,
            sat,
        } => {
            let x = m.freg(a);
            let x = m.modify(x, abs, neg);
            let v = match op {
                MufuOp::Cos => m.ext(float, GLSL_COS, &[x]),
                MufuOp::Sin => m.ext(float, GLSL_SIN, &[x]),
                MufuOp::Ex2 => m.ext(float, GLSL_EXP2, &[x]),
                MufuOp::Lg2 => m.ext(float, GLSL_LOG2, &[x]),
                MufuOp::Rcp => {
                    let one = m.fconst(1.0);
                    m.op(OP_F_DIV, float, &[one, x])
                }
                MufuOp::Rsq => m.ext(float, GLSL_INVERSE_SQRT, &[x]),
                MufuOp::Sqrt => m.ext(float, GLSL_SQRT, &[x]),
                MufuOp::Rcp64h | MufuOp::Rsq64h => return Err(unsupported),
            };
            let v = m.saturate(v, sat);
            m.fset(d, v)
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
            let mut x = m.reg(a);
            let mut y = m.operand(at, b, false)?;
            if neg_a {
                x = m.op(OP_S_NEGATE, uint, &[x]);
            }
            if neg_b {
                y = m.op(OP_S_NEGATE, uint, &[y]);
            }
            let v = m.op(OP_I_ADD, uint, &[x, y]);
            m.set(d, v)
        }
        Instr::Fcmp { d, a, b, c, cmp } => {
            let z = m.foperand(at, c)?;
            let zero = m.fconst(0.0);
            let r = m.compare(at, cmp, z, zero)?;
            let av = m.reg(a);
            let bv = m.operand(at, b, true)?;
            let v = m.op(OP_SELECT, uint, &[r, av, bv]);
            m.set(d, v)
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
            let x = m.freg(a);
            let x = m.modify(x, abs_a, neg_a);
            let y = m.foperand(at, b)?;
            let y = m.modify(y, abs_b, neg_b);
            let r = m.compare(at, cmp, x, y)?;
            let cv = m.pred(c);
            let v = m.bool_op(at, bop, r, cv)?;
            let on = m.uconst(if float_result {
                1.0f32.to_bits()
            } else {
                u32::MAX
            });
            let off = m.uconst(0);
            let out = m.op(OP_SELECT, uint, &[v, on, off]);
            m.set(d, out)
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
            let mut v = m.attribute(at, attr)?;
            if mode == 1 {
                let k = m.freg(multiplier);
                v = m.op(OP_F_MUL, float, &[v, k]);
            }
            let v = m.saturate(v, sat);
            m.fset(d, v)
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
            let half = |m: &mut Module, v: u32, high: bool, signed: bool| -> u32 {
                let at = m.uconst(if high { 16 } else { 0 });
                let w = m.uconst(16);
                let op = if signed {
                    OP_BIT_FIELD_S_EXTRACT
                } else {
                    OP_BIT_FIELD_U_EXTRACT
                };
                m.op(op, uint, &[v, at, w])
            };
            let bv = match b {
                Source::Imm20(i) => m.uconst(i & 0xffff),
                _ => m.operand(at, b, false)?,
            };
            let av = m.reg(a);
            let ha = half(m, av, high_a, sign_a);
            let hb = half(m, bv, high_b, sign_b);
            let mut product = m.op(OP_I_MUL, uint, &[ha, hb]);
            let sixteen = m.uconst(16);
            if psl {
                product = m.op(OP_SHIFT_LEFT_LOGICAL, uint, &[product, sixteen]);
            }
            let cv = m.reg(c);
            let addend = match mode {
                XmadMode::Full => cv,
                XmadMode::Lo => {
                    let mask = m.uconst(0xffff);
                    m.op(OP_BITWISE_AND, uint, &[cv, mask])
                }
                XmadMode::Hi => m.op(OP_SHIFT_RIGHT_LOGICAL, uint, &[cv, sixteen]),
                XmadMode::Bcc => {
                    let s = m.op(OP_SHIFT_LEFT_LOGICAL, uint, &[bv, sixteen]);
                    m.op(OP_I_ADD, uint, &[cv, s])
                }
                XmadMode::Csfu => unreachable!(),
            };
            let mut v = m.op(OP_I_ADD, uint, &[product, addend]);
            if mrg {
                let mask = m.uconst(0xffff);
                let lo = m.op(OP_BITWISE_AND, uint, &[v, mask]);
                let hi = m.op(OP_SHIFT_LEFT_LOGICAL, uint, &[bv, sixteen]);
                v = m.op(OP_BITWISE_OR, uint, &[lo, hi]);
            }
            m.set(d, v)
        }
        Instr::Texs {
            d0,
            d1,
            a,
            b,
            handle,
            kind: 2,
            mask,
        } => {
            let comps = crate::exec::texs_components(mask, d1 != RZ).ok_or(unsupported.clone())?;
            if d0 == RZ {
                return Err(unsupported);
            }
            let x = m.freg(a);
            let y = m.freg(b);
            let texel = m.sample_lz(handle, x, y, None);
            let targets = [d0, d0.saturating_add(1), d1, d1.saturating_add(1)];
            for (i, c) in comps.iter().enumerate() {
                let v = m.op(OP_COMPOSITE_EXTRACT, float, &[texel, *c as u32]);
                m.fset(targets[i], v);
            }
        }
        Instr::Tex {
            d,
            a,
            b,
            handle,
            tex_type: 2,
            mask,
            dc: false,
            aoffi,
            blod: 1,
            ndv: false,
            ..
        } if d != RZ && a != RZ && (!aoffi || b != RZ) => {
            let offset = if aoffi { Some(m.reg(b)) } else { None };
            let x = m.freg(a);
            let y = m.freg(a.saturating_add(1));
            let texel = m.sample_lz(handle, x, y, offset);
            let mut target = d;
            for c in 0..4u32 {
                if mask & (1 << c) != 0 {
                    let v = m.op(OP_COMPOSITE_EXTRACT, float, &[texel, c]);
                    m.fset(target, v);
                    target = target.saturating_add(1);
                }
            }
        }
        _ => return Err(unsupported),
    }
    Ok(())
}
