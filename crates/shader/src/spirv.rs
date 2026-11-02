//! Direct SPIR-V 1.0 emission; no runtime GLSL compiler or external crate.
use crate::{Error, Op, Program, Stage, Type, MAX_SPIRV_WORDS};
struct Builder {
    next: u32,
    declarations: Vec<u32>,
    annotations: Vec<u32>,
    body: Vec<u32>,
}
impl Builder {
    fn id(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }
    fn declare(&mut self, op: u32, args: &[u32]) -> u32 {
        let id = self.id();
        let mut operands = vec![id];
        operands.extend(args);
        instruction(&mut self.declarations, op, &operands);
        id
    }
    fn result(&mut self, op: u32, ty: u32, args: &[u32]) -> u32 {
        let id = self.id();
        let mut operands = vec![ty, id];
        operands.extend(args);
        instruction(&mut self.body, op, &operands);
        id
    }
}
fn instruction(out: &mut Vec<u32>, op: u32, args: &[u32]) {
    out.push(((args.len() as u32 + 1) << 16) | op);
    out.extend_from_slice(args);
}
/// Program is constructed only by checked decoding. All host output is owned;
/// even a budget failure returns no partial module to the caller.
pub fn emit(program: &Program) -> Result<Vec<u32>, Error> {
    let mut b = Builder {
        next: 1,
        declarations: vec![],
        annotations: vec![],
        body: vec![],
    };
    let void = b.declare(19, &[]);
    let uint = b.declare(21, &[32, 0]);
    let float = b.declare(22, &[32]);
    let vec4 = b.declare(23, &[float, 4]);
    let ptr_out = b.declare(32, &[3, vec4]);
    let function_type = b.declare(33, &[void]);
    let output = b.id();
    instruction(&mut b.declarations, 59, &[ptr_out, output, 3]);
    let input = if program.stage == Stage::Vertex {
        let ptr = b.declare(32, &[1, vec4]);
        let var = b.id();
        instruction(&mut b.declarations, 59, &[ptr, var, 1]);
        instruction(&mut b.annotations, 71, &[var, 30, 0]);
        Some(var)
    } else {
        None
    };
    if program.stage == Stage::Vertex {
        instruction(&mut b.annotations, 71, &[output, 11, 0]);
    } else {
        instruction(&mut b.annotations, 71, &[output, 30, 0]);
    }
    let main = b.id();
    let label = b.id();
    instruction(&mut b.body, 54, &[void, main, 0, function_type]);
    instruction(&mut b.body, 248, &[label]);
    let loaded = input.map(|i| b.result(61, vec4, &[i]));
    let mut ids = Vec::with_capacity(program.values.len());
    for value in &program.values {
        let id = match value.op {
            Op::Constant(x) => {
                let id = b.id();
                instruction(&mut b.declarations, 43, &[uint, id, x]);
                id
            }
            Op::Input(c) => {
                let f = b.result(
                    81,
                    float,
                    &[loaded.expect("validated vertex interface"), u32::from(c)],
                );
                b.result(124, uint, &[f])
            }
            Op::BitsToFloat(a) => b.result(124, float, &[ids[a]]),
            Op::FloatToBits(a) => b.result(124, uint, &[ids[a]]),
            Op::Add(a, c) | Op::Multiply(a, c) => {
                let id = b.result(
                    if matches!(value.op, Op::Add(..)) {
                        129
                    } else {
                        133
                    },
                    float,
                    &[ids[a], ids[c]],
                );
                instruction(&mut b.annotations, 71, &[id, 42]); // NoContraction
                id
            }
        };
        debug_assert!(matches!(value.ty, Type::Bits32 | Type::F32));
        ids.push(id);
    }
    let outputs = program.outputs.map(|i| b.result(124, float, &[ids[i]]));
    let vector = b.result(80, vec4, &outputs);
    instruction(&mut b.body, 62, &[output, vector]);
    instruction(&mut b.body, 253, &[]);
    instruction(&mut b.body, 56, &[]);
    let mut module = vec![0x07230203, 0x00010000, 0, b.next, 0];
    instruction(&mut module, 17, &[1]); // Shader
    instruction(&mut module, 14, &[0, 1]); // Logical, GLSL450
    let mut entry = vec![
        if program.stage == Stage::Vertex { 0 } else { 4 },
        main,
        0x6e69616d,
        0,
        output,
    ];
    if let Some(input) = input {
        entry.push(input);
    }
    instruction(&mut module, 15, &entry);
    if program.stage == Stage::Fragment {
        instruction(&mut module, 16, &[main, 7]);
    } // OriginUpperLeft
    module.extend(b.annotations);
    module.extend(b.declarations);
    module.extend(b.body);
    if module.len() > MAX_SPIRV_WORDS {
        return Err(Error::Budget);
    }
    Ok(module)
}
