# Inset texture investigation (0.27.2)

Two successive physical Mali reports reproduce the same inset failure:
239/240 diagnostics and 54/55 patterns. The reports contain no differing pixel
coordinates or values, so they cannot yet distinguish interpolation, sampling
or another rendering error. The original case and strict byte comparison remain
unchanged. No guest shader, rasterizer, command page or Rust ABI changes here.

Every captured guest replay now records a bounded `comparison`. Color and D32
attachments are both compared, even when one fails. A failed attachment includes
the complete expected/actual image in row-major hexadecimal byte order, its
size, mismatch count and the first eight differing `(x,y)` coordinates/values.
Successful attachments retain just dimensions, format and zero differences.
Images are bounded to64x64; oversized actual data is truncated explicitly and
size mismatches always fail. Depth bytes retain little-endian float32 bits.
The eight-coordinate sample limit keeps a synthesized current report with all 185
attachments failing below the existing 1 MiB cache-read limit (850,382 bytes), while
retaining every expected/actual image byte.
The visible scene preserves this evidence before failed native resources close;
the existing automatic JSON export carries it unchanged.

Twelve extra native controls run automatically in the existing one-button suite:

| Controls | What they vary |
| --- | --- |
| Unit W, uniform W=2 | Same projected rectangle and UVs with constant W |
| Reverse winding | Both triangles reverse position and varying order |
| U minus/plus, V minus/plus, UV minus/plus | Dyadic1/64 normalized offsets, with an independent exact texel oracle |
| Repeat original | Original captured inset workload again, after controls |
| Variable-W and unit-W UV readback | Original translated vertex shader, diagnostic fragment output to float32 attachment before sampling |

The ten color controls compare every byte against independent rectangle/texel
formulas. They are host diagnostic copies, not new emulated guest features.
The two UV probes record all 128 actual/reference uint32 bit pairs, maximum
absolute error and unexpected-pixel count. They require exact coverage and Z/W
output components, with an explicit0.0001 absolute bound for covered U/V values.
That probe bound never affects any guest texture/color/depth acceptance rule.
An unsupported float attachment records a failed probe and continues; capability
support is queried before use. The regular renderer continues to use its original
fragment shader and nearest sampler. The probe's substituted fragment shader is
diagnostic only and never replaces a failed texture result.

The GPU report also records Vulkan subpixel, subtexel and mipmap precision bits.
Vulkan specifies separate requirements for non-shader computation and
[sampling precision](https://docs.vulkan.org/spec/latest/chapters/textures.html).
[NoPerspective fragment inputs use linear interpolation](https://docs.vulkan.org/spec/latest/chapters/shaders.html).
Texel-boundary precision is still only a hypothesis; the next physical report
must provide actual values before changing the rendering contract.

App 0.27.2/code 32/schema 4/workload 31/native 16 runs252 diagnostics:
engine18/NV50/GPU-core76/shader25/Vulkan83. The same 55 ordered visible patterns
remain. Updating the workload invalidates stale cached diagnostics. The private
in-app updater and all 21 existing Rust exports/layouts remain intact.
Commercial games and broader perspective/clipping/runtime work remain open.
