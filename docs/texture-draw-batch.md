# Guest texture and varying batch (0.21)

The original shader, GPU reference executor and Vulkan replay now support a
bounded textured-geometry path. The library includes **Texture scene**, with
three variants rendered from owned guest command pages and shader programs.
The first variant executes `nv-texture.nro` through the original A64 interpreter,
kernel, NV IPC and GPU parser before replaying its captured draw inputs through
Vulkan. The other variants exercise BGRA target storage and changed texture data.
This is not general imported-homebrew presentation or commercial game support.

## Shader interface

The vertex profile can read two vec4 float inputs at generic attributes 0/1
(bytes 0x80/0x90), write Position and export generic0 through AST. Fragment
`IPA.PASS` reads generic0 at pixel centers with screen-linear interpolation.
The SPH declares the actual input/output maps; unsupported header fields and
incomplete exports fail. The original immutable SSA and serial evaluator retain
separate position and varying outputs. SPIR-V uses location0/1 vertex inputs,
location0 varyings and NoPerspective interpolation.

The accepted texture operation is unconditional `TEX.B.LZ`, 2D, all RGBA channels,
without modifiers. Its handle must resolve to an SSA constant and every texture
instruction in one program must use that same handle. Dynamic handles, implicit
LOD/derivatives, other dimensions, offsets, predication and alternate channel
masks fail. The four scalar sample results preserve source operands before
overwriting destination registers. SPIR-V emits an explicit zero-LOD sampled
image at descriptor set0/binding0. Sampler state is supplied by validated guest
descriptors, not selected by a title-specific driver setting.

`SampledImage` is a checked borrowed view of tightly packed RGBA bytes, bounded
to 64×64. The scalar sampler implements normalized coordinates, nearest sampling
and clamp-to-edge; NaN/infinite coordinates fail. Shader invocation returns
explicit errors for absent/malformed images or unsupported interfaces. The old
single-input evaluator rejects programs needing the second vertex input.

The authored SASS encodings/listings match NVIDIA nvdisasm in SM53 binary mode.
All four new emitted modules pass Khronos SPIR-V validation for Vulkan1.1.
Instruction encodings were researched from [primary Maxwell hardware research](https://github.com/envytools/envytools/blob/master/envydis/gm107.c)
and independently checked; no emulator implementation was copied.

## Guest descriptors and draw preparation

Methods use [NVIDIA's B197 interface](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/3d/clb197.h).
The Maxwell-header selector is explicit. Texture and sampler pools use their
guest GPU addresses and maximum indices; pools must be 256-byte aligned. The
constant handle splits into a 20-bit texture index and 12-bit sampler index,
checked against the configured bounds. Both 32-byte descriptors are captured.

The [public Maxwell texture/sampler layouts](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/3d/clb197tex.h)
define this supported profile: pitch-linear 2D RGBA8 UNORM, identity RGBA swizzle,
normalized coordinates, depth1, no mipmaps/sRGB, nearest min/mag, clamp on all
axes, no compare/aniso/bias. Other fields/reserved bits fail. Texture dimensions
are 1..64, pitch is 32-byte aligned, at least width×4 and at most256. The original
40-bit GPU address rules apply. Padded rows are captured and repacked for host
upload. Tiled textures, compressed guest formats and broader sampler modes
remain open; the older standalone BC1 conversion workload is separate.

Vertex stream0 uses stride32 when the second input is consumed, with attribute1
at offset16. The existing position domain remains explicit: finite coordinates,
w1, z0..1, unclipped x/y and a reversible 1/16-pixel grid. Rasterization uses the
existing pixel-center/top-left rules; screen-linear attributes are evaluated per
covered fragment. Draws publish prepared pixel rows in command order. Uniform
legacy shaders retain a constant-result path. No depth, blending, perspective
division or clipping is approximated.

Shader, vertex, TIC/TSC and texture data are owned snapshots backed by retained
pins. Any batch write overlapping captured inputs fails by backing identity,
including GPU aliases and earlier/later DMA or draws. Unmapping and reusing CPU/
GPU addresses cannot redirect a prepared job. Between jobs, changed texture bytes
are fetched again; no texture cache is implemented. The existing operation,
snapshot/effect-byte and backing-span budgets remain enforced.

## Reference, ABI and display

The shared owned NV guest harness now has a texture profile. The original draw
NRO binary remains unchanged. `nv-texture.nro` executes5571 instructions/25 IPC,
rejects a late bad method before pixels/fence publication, draws two textured
quads, closes bound owners, redraws from retained state and verifies completion2.
Host formulas independently check all128 pixels and source preservation. The
4×4 source palette is unrelated to a console game or third-party asset.

New value ABI `emulator_texture_replay_v1`: a 99,928-byte initialized owned result,
alignment4; three 33,028-byte TexturedDraw records contain the unchanged DrawOp,
six vec4 varying inputs and a bounded RGBA texture. Offsets are checked in Rust/
C++. Failure leaves status nonzero and zero published draw/image counts. All16
previous C exports and their layouts remain unchanged.

Vulkan uploads each captured texture, binds its sampler and both vertex inputs,
executes our translated modules and waits for completion. Every framebuffer byte
must match the independent reference before it can be presented. The scene uses
the existing Android swapchain owner, preserves the source aspect ratio, and
retains a destroyed TextureView until its worker releases Vulkan. Superseded
queued requests are skipped. Backgrounding closes the scene; returning recreates
it. Pattern/rotation/reopen support reports are cached atomically and included
in the ordinary diagnostic export.

The batch is now121 checks: engine16, NV50, GPU core11, shader25 and Vulkan19.
Report schema4/workload22 invalidates earlier caches; native workload8 changes
with the expanded corpus. The previous103 named test records remain exact.
There are no new external Cargo/runtime dependencies. General VI/HID/display
queues, commercial content, broader ISA/shaders/textures and performance remain
unfinished; this is a development checkpoint.
