# Interpolated guest depth (0.25)

The physical [0.24 depth report](validation/g720-depth.md) confirms all 168
diagnostics and 21 patterns on Mali. This extension removes the flat-Z-per-triangle
restriction and adds two sloped surfaces, while retaining the existing w=1,
finite/unclipped position, 1/16-pixel XY grid and single-GOB ZF32 limits.

The reference rasterizer interpolates each covered pixel's depth from the three
post-vertex-shader Z values and integer edge-function barycentric weights.
Its numerical policy is an f64 weighted sum/division, rounded once to f32 before
comparison or storage. Vertex depth travels with its vertex when winding is
normalized. Degenerate triangles produce no fragments, scissor excludes depth
effects, and disabled writes preserve the original attachment. Existing ordered
snapshot, feedback, permissions, bounds, cancellation and budget rules apply.

This implements screen-linear depth interpolation in the existing w=1 subset,
consistent with the [Khronos rasterization contract](https://docs.vulkan.org/spec/latest/chapters/primsrast.html).
It does not implement perspective divide, clipping or arbitrary-w geometry.
Different hardware may use different permitted interpolation precision for
general data; universal bit identity is not claimed. The new oracle corpus uses
exactly representable dyadic planes with pixel-center depths `(2*x+1)/32` and
`1-(2*x+1)/32`, so these workloads require exact D32 bit comparisons.

Both cases render an angled green surface and an overlapping flat red rectangle.
Only the right half of that rectangle passes depth. The second case reverses
the slope and uses greater comparison. Independent integer coordinate formulas
predict every displayed color and stored depth without running shader/raster
decoders to generate the expected result. Vulkan executes the captured vertex
Z values, compares actual color/depth readback and then presents the image.

The existing `DepthReplay` layout, ABI version and all 20 value exports are
retained. Fixture indices 9 and 10 extend the owned replay corpus; the previous
nine depth pages and all previous diagnostic records remain unchanged. Two new
GPU-core groups cover the plane oracles, reversed winding, degenerate primitives
and disabled writes. Rust integration checks also cover scissor and every D32
word using the independent general GOB decoder.

App/code/schema/workload/native versions are 0.25.0-dev / 28 / 4 / 28 / 13.
The single **Test all & export JSON** run now has **172 diagnostics and 23
patterns**: engine 18, NV 50, GPU core 40 (suite 7), shader 25 and Vulkan 39.
No new permissions or game-loading capabilities are added. The new physical
Mali checkpoint is pending. Depth clears, larger layouts, perspective/clipping,
broader shaders/runtime and general live guest depth/display remain open.
