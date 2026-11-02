# Indexed guest geometry (0.22)

The original B197 guest renderer now captures index buffers and reuses vertices
through both the scalar rasterizer and Vulkan. The library's Texture scene has
six patterns: the previous three textures, a byte-index quad, a u16 quad with a
negative base vertex and a u32 pattern that draws only one inset triangle.
These are authored reference workloads. Imported programs still use the bounded
text runner; commercial games and general guest presentation remain open.

## Wire profile

Numeric fields were checked against NVIDIA's primary
[MAXWELL_B class header](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/3d/clb197.h).
The original decoder accepts packed `DRAW_INDEX_BUFFER32/16/8_BEGIN_END_INSTANCE_FIRST`
at 0x17e4/0x17e8/0x17ec. FIRST occupies bits 15:0, COUNT bits 27:16 and TOPOLOGY
bits 31:28; only triangle lists (4) are implemented. The explicit method selects
4/2/1-byte elements. This slice deliberately rejects state-driven 0x17e0 draws,
INDEX_BUFFER_E/F, subsequent instances, strips, restart and instancing.

SET_INDEX_BUFFER_A..D at 0x17c8..0x17d4 provide the 40-bit start and inclusive
limit. SET_GLOBAL_BASE_VERTEX_INDEX at 0x1434 is consumed as a signed base;
SET_DA_PRIMITIVE_RESTART_VERTEX_ARRAY at 0xde8 must explicitly be zero. Signed
index bias follows the [Gallium draw contract](https://docs.mesa3d.org/gallium/context.html). Both are
required for indexed draws. Selected element bytes must be naturally aligned,
readable, mapped and within the limit. First-index slicing captures only selected
elements, preserving their raw values. Base addition uses i64 and rejects
negative or >=96 effective indices, without wrapping. Counts are 1..96 and a
multiple of three. A dense 0..maximum-effective vertex window is captured within
the declared vertex limit; unreferenced vertices are not shader-invoked.

Depth, perspective, clipping, culling and blending remain outside the guest
profile. The previous finite w=1, reversible 1/16-pixel grid, <=64×64 target,
screen-linear varying and nearest/clamp RGBA texture limits still apply.

## Transactions, ownership and Vulkan

Immutable element values and pins are retained with shader/vertex/texture inputs.
Decoded index storage is charged in addition to captured raw bytes, within the
existing 4MiB snapshot/effect and 4096-span budgets. All-batch input/output
feedback rejects overlapping backing identities, including GPU aliases. A bad
second draw or final command cannot publish preceding clears/pixels/channel state.
Unmap/address reuse cannot redirect prepared effects. Subsequent submissions
recapture changed elements from retained registers.

The scalar rasterizer consumes element order, repeated corners and either winding.
The Vulkan adapter binds an index buffer, issues `vkCmdDrawIndexed`, and preserves
the signed base vertex. Selected first-index slicing is already reflected in the
captured buffer, so Vulkan firstIndex is zero. Byte elements widen exactly to u16;
no optional uint8-index extension is required. u16/u32 retain their widths.
Host writes become visible to index reads with explicit barriers, and all work
completes before readback/presentation/resource release. Full readback must match
the independent pixel formula, including the missing half of the u32 inset.

## Original guest and value bridge

`nv-indexed.S` shares only our own original NV assembly harness. Its byte-index
page is interpreted through A64/kernel/HIPC/NV, rejects a late bad command, checks
pixels/fences, closes backing owners and submits again using retained state.
It exits after 5571 instructions, 25 IPCs, 18 A/11 B descriptors, 603 buffer bytes
and two copied handles. Completion is 2; handles, transfer objects and loans end
at zero. All earlier NRO binaries remain byte-identical.

`emulator_indexed_replay_v1` is a new value-only export. IndexedDraw is 33,424
bytes: the unchanged TexturedDraw followed by count/element width/signed base and
96 u32 elements. IndexedReplay is 101,116 bytes, alignment 4, with draws at 76,
pixels at 100,348 and error at 100,860. Existing 17 exports/layouts remain stable;
older converters explicitly reject indexed state. The first replay executes and
captures the owned NRO; other cases use authored pages. This is synchronous
diagnostic replay, not a general guest display queue.

The portable report adds nine groups: formats/offsets/base; invalid bounds/modes;
late-command rollback; direct/alias feedback; immutable capture/unmap/reuse;
changed retained elements; unreadable/unmapped/partial inputs; second-draw
rollback; and NRO lifetime. Three native Vulkan cases bring the combined suite
to 134 (17 engine, 50 NV, 20 GPU core, 25 shader, 22 Vulkan). Host tests additionally
check budgets/cancellation, winding/unused inputs and unsupported setup rollback.
Target-phone confirmation of this addition is pending; the prior 121-check
[texture checkpoint](validation/g720-texture-batch.md) is physically confirmed.
