# Original SM53 shader translation batch

0.17.0-dev adds the dependency-free `emulator-shader` crate, independent of GPU
mappings, kernel services, Android and Vulkan. It decodes raw Maxwell SM53
instruction bytes into immutable typed SSA, evaluates a reference invocation,
and emits SPIR-V 1.0 directly. No runtime GLSL compiler, external Rust crate,
third-party translator or existing emulator core is linked/copied.

This is an offscreen owned-shader checkpoint. Host code supplies stage metadata,
vertex buffers, pipelines and draw calls. Graphics program headers and guest draw
methods do not feed this translator yet. The M4 gate requiring a guest executable
through services, commands/shaders and presentation remains open.

## Exact supported input

Complete little-endian 32-byte bundles: one 64-bit scheduling word, three 64-bit
instructions. At most 2048 code bytes (64 bundles / 192 instruction slots).
Scheduling bits 0..62 describe stall/barrier/reuse hints and do not affect serial
semantics for these straight-line register/attribute operations. Reserved bit63
fails. There is no scheduler, branch, memory visibility or latency model here.

All instructions require PT (unconditional), with only these exact forms:

- MOV32I, all byte lanes: preserve the immediate's entire 32-bit pattern.
- MOV register, all byte lanes: preserve an earlier register's value.
- FADD / FMUL register, default round-to-nearest, no modifiers/flags: explicit
  F32 operations, with SPIR-V NoContraction. No fused arithmetic is introduced.
- ALD, vertex input attribute bytes 0x80..0x8f (host location 0), 1..4 components.
- AST, vertex position bytes 0x70..0x7f (host Position), 1..4 components.
- EXIT, unconditional; remaining slots must be exact NOP padding.

Attribute base/vertex operands must both be RZ, no indirect addressing or other
flags. Accesses must be aligned and stay inside the one vec4; vector register
ranges cannot reach RZ. R0..R254 start undefined; reads before definition fail.
RZ reads zero and discards writes. Source values are captured before overwriting
a destination. A vertex program must write all four position components. The
explicit fragment interface returns R0..R3 as location-0 vec4 at EXIT, all defined.
Missing EXIT/output, unsupported predication, modifiers, attributes, opcodes,
branches, shader stages and malformed inputs fail without a partial Program.

Float arithmetic references and GPU fixtures establish the tested finite normal
values only. Denormals, NaN payload arithmetic, saturation/FTZ/rounding modifiers,
exceptions, derivatives, textures, interpolation/varyings, constants/uniforms,
shader memory and control flow remain unsupported or unvalidated. Bit-move CPU
oracles preserve signed zeros, subnormals, infinities and NaN bit patterns; these
are not a claim about corresponding arithmetic or render-target conversion.

IR values explicitly carry Bits32/F32 types and immutable operands. Bitcasts
surround arithmetic. Values/output indices cannot be supplied by callers; a
Program comes from validated decode. Each emitted SPIR-V module is bounded to
2048 words; expansion overflow returns an error with no partial module.

## Owned corpus and independent tools

`examples/shaders/` contains two original vertex fixtures and green/red fragment
fixtures, both raw code and readable SASS listings. The generator authors the
bytes; no vendor or game shader is bundled. The four complete listings (including
padding) match NVIDIA nvdisasm 12.8.55 in SM53 binary mode exactly. Encodings were
researched from public hardware/disassembly facts and checked using NVIDIA's
independent disassembler, not another emulator.

Sources:
[NVIDIA Binary Utilities](https://docs.nvidia.com/cuda/archive/12.8.0/cuda-binary-utilities/index.html),
[NVIDIA redistribution manifest](https://developer.download.nvidia.com/compute/cuda/redist/redistrib_12.8.0.json),
[envytools Maxwell hardware research](https://github.com/envytools/envytools/blob/master/envydis/gm107.c),
[Khronos SPIR-V grammar](https://github.com/KhronosGroup/SPIRV-Headers/blob/main/include/spirv/unified1/spirv.core.grammar.json).
The independently downloaded nvdisasm archive matched the manifest SHA-256
`102b74439c32f5be7be8aa983aa0251045cf52cb75a05f4ffe32e13c7480090a`.
Research files/tools are ignored, absent from build/runtime dependencies.

```sh
python3 tools/generate_shader_fixtures.py --check --validate-spirv
# Optional additional independent binary verification:
python3 tools/generate_shader_fixtures.py --check --nvdisasm /path/to/nvdisasm
cargo test --offline -p emulator-shader
```

The validation flag compiles our four fixtures with our Rust emitter and runs
Khronos spirv-val for Vulkan 1.1. Mutation tests flip every single bit of all four
fixtures (2048 mutants) and require bounded rejection or a valid owned plan;
all truncations are checked. The fifteen portable report cases include actual
instruction/module/invocation/rejection counts, not predetermined pass flags.

## Value ABI and Vulkan evidence path

`emulator_run_shader_checks_v2()` returns the existing bounded EngineReport ABI 2
(8200 bytes, 8192-byte JSON payload). `emulator_shader_draw_v1(index)` returns an
owned 17264-byte/alignment-4 value: ABI/status/vertex-word-count/fragment-word-count,
two 2048-word module arrays, 24 vertex float bit patterns, 512 expected RGBA image
bytes and 256 error bytes. Offsets: modules 16/8208, vertices 16400, pixels 16496,
error 17008. Rust/C++ assert the layout. Unknown index/panic returns failure and
zero word counts; initialized output contains no pointers or callbacks.

The three Vulkan workloads use these emitted modules, not fixed host shaders:

1. Passthrough vertex + green fragment: all 16x8 pixels exactly match green.
2. FMUL-transformed vertex + red fragment into BGRA: centered 8x4 rectangle,
   opaque black border and red/blue storage order all match independent pixels.
3. Two translated fragment pipelines: full green then left-half red using scissor
   in one submission, verifying shader content changes and ordered coverage.

Vertex inputs are two triangles covering the viewport. CPU image expectations
are direct rectangle/color formulas, independent of the SSA evaluator/emitter.
Each target/readback has explicit render/transfer/host dependencies and a waited
fence before comparison. Native resource owners retain pipelines/modules/buffers
until completion. This does not implement asynchronous guest fences or display.

Schema 4/workload 17 now requires five groups: engine14, NV34, GPU-core3,
shader15 (suite1), Vulkan12 (workload5), total78. Summary derives actual counts;
one Run all checks runs/exports every group. Workload16 caches are invalidated.
