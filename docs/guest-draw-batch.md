# Original guest shader and draw command batch

0.18.0-dev connects actual B197 state and GPU virtual addresses to the original
shader decoder, typed SSA and SPIR-V emitter. A new interpreted NRO uses NV IPC,
GPFIFO commands, 80-byte program headers, shader bodies and vertex data, submits
triangles and checks pixels/fences/rollback before closing resources. The CPU
reference renderer commits pixels transactionally. Vulkan separately replays the
same captured guest command page through the same parser and translated shaders.

The host now derives shader selection, vertices, target and scissor from guest
commands for these three new replays. It still supplies the Vulkan API lowering;
no live asynchronous rendering, display-buffer handoff or Android swapchain is
implemented. This is meaningful partial M4 progress, not its presentation gate.
No commercial title or console performance claim follows.

## Header profile and program capture

The original shader crate adds `decode_program`, requiring an80-byte SPH and
complete raw SM53 bundles. It supports one explicit profile: Version3/SASS1,
VERTEX/VTG1 or PIXEL/PS2, no local memory/stacks/patches/MRT/kills/global stores/
FP64/stream-out. StoreReqStart1 > End0 disables forced exports. Vertex declares
one vec4 generic input and Position.xyzw; pixel declares target0 RGBA only.
All other fields/flags/maps/reserved bits fail. Vertex enables DoesLoadOrStore.
This does not parse every valid NVIDIA/SPH or SDK container profile.

Word0: vertex0x04020461 or pixel0x00021462; word4:0x1000. Vertex word6:0xf,
word13:0xf000; pixel word18:0xf. All other words zero. These are original fixture
headers based on the field widths/order and enumerations in
[NVIDIA's SPH specification](https://nvidia.github.io/open-gpu-doc/Shader-Program-Header/Shader-Program-Header.html).
The introductory prose reverses type descriptions; the implementation follows
the CommonWord0 enumeration (VTG1/PS2) and corresponding bit-layout tables.
No emulator header/parser implementation is used.

Program region + pipeline offset must be256-byte aligned, with checked40-bit
GPU memory. The GPU executor captures each header and32-byte bundles through the
first unconditional EXIT, at most2048 code bytes. Unterminated/malformed/unmapped
programs fail. Shader header stage, pipeline type, explicit register allocation
and actual required registers must agree. Four added portable cases check both
headers, all1280 single-bit header mutations and every truncation/stage mismatch.
The existing raw shader subset and its2048 bit mutations remain unchanged.

## Draw state and reference execution

Method fields come from
[NVIDIA's public B197 interface](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/3d/clb197.h).
This deliberately narrow state is explicit; missing required values fail:

- Region0x1608/160c, vertex pipeline1 and pixel pipeline5 control/program/register
  methods at0x2040/2044/204c and0x2140/2144/214c. Other four pipeline controls0.
- Vertex stream0 at0x1c00..1c0c: enabled, stride16, no frequency/instance advance;
  attribute0 at0x1160: vec4 float32/stream0/offset0, with inclusive limit1f00/1f04.
- Draw0x1214: BEGIN_END_INSTANCE_FIRST, triangle list, count1..96/multiple3,
  bounded first vertex. No indexed, strip/fan, instancing or retained open begin.
- Same pitch-linear target0 RGBA8/BGRA8 surface as the clear subset; draw width/
  height at most64 each. CT_SELECT1, CT_WRITE0x1111, no depth/stencil/blend/cull.
- Viewport scale(width/2,height/2,1), offset(width/2,height/2,0), matching zero-origin
  viewport clip/z0..1, upper-left window/no flip, scale-offset enabled. Normal
  scissor independently clips draws; clear-rectangle enable does not affect them.

Vertex inputs are captured before evaluating the original vertex program.
Positions must be finite, w1, z0..1 and x/y inside[-1,1]. Screen positions must be
on an exactly reversible1/16-pixel grid; tiny off-grid coordinates cannot become
accepted through viewport rounding. Clipping/perspective/general numerical
behavior remains open. The scalar reference rasterizer uses pixel centers,
integer edge equations, top-left ownership and either winding; zero-area triangles
produce no fragments. Uniform fragment color allows a coverage union per draw.
No interpolation, textures, depth or blending are silently approximated.

Prepared row effects preserve method order with clears and DMA. All shader/
vertex bytes and emitted modules are owned snapshots; input/target pins keep the
original backing through CPU unmap/address reuse. Failed commands/replies cancel
pixels, state, resources and fences. GPU/B output and active TLS aliases fail
before commit. Any batch output overlapping captured shader/vertex inputs is an
explicit unsupported feedback error, checked using backing identity even across
GPU aliases. This covers earlier and later writes; self-modifying shader/vertex
batches require a future ordered fetch model.

The existing64-operation/4MiB effect budget now includes draws and captured
input/module bytes, with at most4096 pinned spans. A draw's written-byte counter
counts unique covered pixel bytes, not hardware fragment invocations. A valid
empty/degenerate draw still counts one operation and completes its job.
Persistent channel state owns independent boxed register values; clones never
share mutable metadata. NV fences still describe synchronous CPU reference
completion, not Vulkan timing.

## Fixtures, protocol cases and Vulkan replay

`tools/generate_draw_fixtures.py` creates three original4096-byte GPU pages and
command binaries (126words): one black clear, full green draw, then an FMUL-scaled
red draw. Variants cover RGBA/BGRA/scissor. Pages contain original SPH/SASS and
six vec4 vertices. Shader offsets0xa00/0xb00/0xc00/0xe00, vertices0xd00,
target0x800, retained-state draw command0x400. A bad trailing method makes a
128-word job reject atomically. The ordinary command page is replayable without
an A64 process; case0 instead reads the actual page from the executed NRO.

`nv-draw.nro` executes5571 instructions/25 IPC/2 copied handles/18 A/11 B/
603 descriptor bytes. It rejects a partial job, submits two draws, checks pixels,
closes bound NVMap/AS fds, redraws from retained state, closes its channel and
checks completion2. Transfer memory marker42 and zero handles/objects/loans are
verified. Independent formulas check all128 pixels. This trace matching the
older clear NRO's instruction count is coincidental; both have separate oracles.

Sixteen additional actual kernel/NV cases cover correct pixels, nonzero first
vertex fetch, formats/scissor, header and shader failures, bad state/domain,
readonly draw targets (clear launch disabled), late/reply rollback, B/TLS aliases,
closed owners/clones, CPU reuse, input feedback, degenerate geometry and budgets.
They do not label host API calls as guest CPU interpretation.

New value ABI `emulator_guest_draw_replay_v1(index)`:50452bytes/alignment4;
header16, background ClearOp60, three DrawOp16536 records, pixels512/error256.
DrawOp has56 bytes of surface/scissor/count fields, two2048-word SPIR-V arrays,
24 vertex input words. Offsets: draws76, image49684, error50196; module/vertex
record offsets56/8248/16440. Rust/C++ check layouts. Failure has status!=0,
zero draws/image size and initialized owned bytes; no pointers/callbacks.

Three new Vulkan workloads consume decoded state and emitted modules, lower the
background clear, then bind each captured vertex buffer/pipeline/scissor and draw.
All512 readback bytes must match the independent rectangle oracle; case0 also
matches the interpreted guest image. This is a replay bridge, not an asynchronous
renderer API; its ABI currently admits at most3 operations with six vertices
per owned fixture. The general reference draw limits above remain separate.

Schema4/workload18 runs102 checks in one tap/export: engine15(suite12), NV50
(suite3), GPU-core3(suite2), shader19(suite2), Vulkan15(workload6). All five groups
must pass. Workload17 caches are invalidated.
