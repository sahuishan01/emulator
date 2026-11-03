# Guest depth testing (0.24)

The original B197 command processor now captures a bounded ZF32 depth attachment,
compares each covered fragment and prepares color/depth writes transactionally.
The Android scene and automatic batch add nine depth patterns. The app is
0.24.0-dev/code 27, schema 4/workload 27/native suite 12: 168 checks
(engine 18, NV 50, GPU core 38, shader 25, Vulkan 37) and 21 visible patterns.

The wire fields follow NVIDIA's public
[MAXWELL_B class header](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/3d/clb197.h):
ZT address/format/block/array pitch at `0xfe0..0xff0`, sizes at
`0x1228..0x1230`, select `0x1538`, layer `0x179c`, read-only `0x7f8`, sparse
`0x1208`, test enable `0x12cc`, write enable `0x12e8`, comparison `0x130c` and
bounds enable `0x19bc`. Both OGL `0x200..0x207` and D3D `1..8` comparison
encodings normalize to never, less, equal, less-or-equal, greater, not-equal,
greater-or-equal and always. Implementation and fixtures are original.

## Supported attachment and raster profile

Only format ZF32 (`0x0a`), one generic 16Bx2 GOB, a 512-byte-aligned 40-bit
address, block size/array pitch zero, third dimension one, selected target one,
layer zero and sparse/bounds disabled are accepted. Depth dimensions equal the
color surface, at most 16 by 8 pixels. Write enable and the explicit Z read-only
bit must agree. A read pin is always required; write-enabled state also requires
a write pin for the complete 512-byte footprint. Used initial depth values must
be finite and in [0,1]. Unused GOB bytes retain their captured values.

Positions retain the existing finite, unclipped w=1 and exact 1/16-pixel XY
grid rules. This first profile requires constant Z within each triangle;
different triangles/draws may have different Z. Varying per-triangle Z fails
explicitly. Depth comparison occurs at covered pixel centers in primitive order.
Only passing fragments update color, and depth changes only when writes are
enabled. Top-left edge ownership, scissor and existing shader color semantics
remain in force. Stencil, depth clears, interpolation/perspective, bias, bounds,
larger/multisampled/layered attachments and other formats remain unsupported.

## Transaction and alias rules

Before reading the first depth attachment, the batch explicitly snapshots all
preceding effects through the backing-aware projection. Later depth draws read
earlier prepared depth writes without touching guest memory. Newly added deferred
effects are resolved incrementally before the next depth read or final plan.
The default path for batches without depth preserves execution-time DMA reads.

The plan retains read/write pins and immutable decoded inputs across CPU/GPU
unmap and address reuse. A write-enabled draw prepares the full GOB, preserving
uncaptured pixel/padding bytes from its ordered input. Its 512-byte write,
512-byte input and decoded depth words join the shared 4 MiB capture/effect
budget. All depth pins join the 4096-span ceiling; launch limits are unchanged.
Budget, permission, shader, later-draw and alias failures cancel everything.

Depth footprints may not overlap non-depth output effects, including color
targets or DMA destinations, even through different backing aliases. General
DMA initialization of a depth attachment within the same batch is therefore
not yet accepted. DMA reads of a completed depth result into a disjoint target
use the ordered snapshot path. Existing shader/vertex/texture input feedback
rejection continues to cover depth outputs. This is a synchronous reference
policy rather than a hardware cache/fence implementation.

## Vulkan replay and visible cases

The twentieth owned value export, `emulator_depth_replay_v1`, returns a
51,500-byte/alignment-4 `DepthReplay`. Its first 50,452 bytes retain the
`GuestDrawReplay` layout; initial/expected linear depth arrays and three
comparison/write states follow. All previous value layouts/exports are retained.
The bridge validates the complete guest-color and tiled-depth results before
returning captured shader modules, vertices and pipeline state.

Vulkan uploads the actual captured initial depth bytes to D32_SFLOAT, loads the
attachment, executes the captured shaders and comparison/write state, and reads
both color and depth back after completion. Every byte/bit must match independent
oracles before a pattern is presented. Transfer/early/late-fragment dependencies
and depth-aspect layout transitions are explicit. Unsupported host formats fail.

Cases cover all eight comparisons, equality at the depth boundary, near and
reverse-depth occlusion, and less comparison with depth writes disabled. The
reference images use authored B197 command pages and the existing original shader
fixtures; they are not nine newly interpreted NRO programs. The old Catch live
callback cannot carry depth metadata and explicitly rejects depth plans. The new
scene replay presents them through its dedicated ABI and the common surface
lifecycle. General live guest display integration remains open.

The existing **Test all & export JSON** button automatically exercises all 21
patterns and 168 diagnostics and saves one report in Downloads/Emulator. A host
or software Android pass does not confirm this new path on physical Mali; the
next supplied 0.24 report is required. Commercial games remain unsupported.
