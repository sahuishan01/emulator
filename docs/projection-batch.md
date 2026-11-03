# Positive-W projection batch (0.27)

The original GPU reference now divides post-vertex-shader clip positions by W.
Captured vertex inputs remain unchanged in the value ABI and are uploaded to
Vulkan, whose fixed-function pipeline performs the actual divide. This extends
our previous w=1 profile; it is not general 3D or commercial-game compatibility.

The profile requires finite positions, positive normal W and a finite normal
float reciprocal. Before division, x/y must be within [-W,W] and z within [0,W].
All six planes are inclusive. Outside vertices fail explicitly; no clipping
algorithm, negative-W handling or silent discard is supplied. Projected XY must
be exactly reversible on the existing1/16-pixel grid. f64 reference division and
reverse reconstruction prevent rounded sums from accepting off-grid values.
Z/W is rounded once to f32 at vertices and interpolated with the existing f64
edge-weight reference before D32 comparison/write. This numerical policy is
explicit; universal hardware bit identity for arbitrary inputs is not promised.

IPA.PASS and its validated SPH linear input map retain screen-linear attributes
and NoPerspective SPIR-V. Variable W must not silently change them into smooth
attributes. Smooth perspective-correct interpolation requires further shader
instructions/interface support. The coordinate transform and interpolation
boundaries follow the [Vulkan coordinate-transform specification](https://docs.vulkan.org/spec/latest/chapters/vertexpostproc.html#vertexpostproc-coord-transform)
and [rasterization specification](https://docs.vulkan.org/spec/latest/chapters/primsrast.html).

Sixteen authored command pages reuse only our own shaders/generators:

| Cases | Inputs and independent checks |
| --- | --- |
| 0..7 | Constant W2/small W, per-corner variable W, inset rectangle, reversed BGRA winding, scissor, nonpower W, subpixel edges; nearest texture coordinate formulas |
| 8..11 | Actual byte/u16/u32 indices, first index, negative/positive base, reversed winding, unused NaN prefix, scissor; rectangle/triangle coordinate formulas |
| 12..15 | Forward/reverse sloped Z/W, read-only comparison, unscaled clip XYZ with W2; complete color and D32 plane formulas |

These are command replays, not sixteen newly interpreted NRO programs. Existing
interpreted homebrew fixtures and31 LLVM fixtures remain unchanged.

Sixteen portable groups report167 verified cases/cycles. They cover all16 image
oracles, normal/nonpower/extreme W, invalid W, six out-of-range clip planes,
off-grid inputs, linear attributes, late invalid second-draw rollback, retained
input ownership across mutation/unmap/reuse, budget/drop/reprepare, changed
resident projection through a retained channel, cancellation/final release,
64 resource cycles, nonfinite referenced XYZ, inclusive clip boundaries,
depth snapshots/read-only preservation and rejection before empty-scissor or
degenerate discard. Unreferenced indexed inputs are intentionally not executed.

No new C exports/layouts: texture replay indices3..10, indexed3..6 and depth11..14
extend the existing value APIs. Old indices and all21 exports stay stable;
first out-of-range IDs fail with zero published draw/image counts.

App0.27.0-dev/code30, schema4/workload30/native15:240 diagnostics
(engine18/NV50/GPU core76 suite9/shader25/Vulkan71),55 visible patterns. The
previous208 named records are unchanged. The one-button workflow includes all
55 patterns and exports one JSON per completed run. New physical Mali
confirmation remains pending; the [0.26 checkpoint](validation/g720-depth-clear.md)
confirms208/39. Clipping, smooth interpolation, larger depth layouts, stencil,
broader shaders/runtime/display and commercial games remain open.
