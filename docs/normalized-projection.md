# Explicit vertex-position projection candidate (0.27.5)

The supplied physical0.27.3 report has246/252 checks and54/55 patterns. Four
inset pixels differ from the independent color oracle. All four actual colors
agree with the nearest texels selected by captured actual UV coordinates, which
lie slightly below reference boundaries. The variable-W probe differs at60 of
128 UV pairs, maximum error4.097819328e-7. Unit-W differs at72 pairs, maximum
3.814697266e-6, yet passes the strict color check. Thus a small UV error alone
does not establish a driver defect, and an increased tolerance cannot repair
texture sampling. No physical target-console evidence establishes universal
bit-exact hardware interpolation here.

Vulkan's [interpolation decorations](https://docs.vulkan.org/spec/latest/chapters/shaders.html#shaders-interpolation-decorations)
make NoPerspective inputs screen-linear; ordinary undecorated inputs use
perspective correction. The [coordinate transformation](https://docs.vulkan.org/spec/latest/chapters/vertexpostproc.html#vertexpostproc-coord-transform)
divides clip XYZ by W. For the currently validated **unclipped, positive-W,
screen-linear** profile, emitting the same projected XYZ with host W=1 preserves
projected geometry and screen-linear attributes mathematically. Actual numerical
behavior still needs strict device testing. This is an inference supporting a
candidate backend, not a guarantee of hardware equivalence.

The original shader emitter now has an optional `emit_normalized_position`
lowering. It executes the original guest vertex program first, including all
guest arithmetic and varying writes, then emits three explicit OpFDiv results
for final XYZ/W and the constant1 for host W. Guest inputs, guest W during
execution, IR, fragment code, sampler, texture, UVs and expected pixels remain
unchanged. No normalized-coordinate offset, epsilon, rounding bias, sampler
change or relaxed color/depth acceptance is introduced. Default `emit` is unchanged.

`Draw::normalized_position_spirv` validates the existing scalar raster profile
before lowering and additionally requires projected XY to be exactly
representable in float32. It rejects indexed and depth-enabled draws for this
candidate milestone. The existing profile already rejects nonfinite/zero/
negative/subnormal W, nonnormal reciprocal, outside-clip and off-grid vertices.
Fragment built-in W, smooth interpolation, clipping, geometry shaders and
transform feedback are not supported. This lowering must not be applied to
those future profiles without a new proof and tests.

A separate pointer-free `emulator_texture_normalized_replay_v1` export returns
the existing TextureReplay layout for the same11 authored texture cases. It
changes only the vertex SPIR-V module/count. Every original export, layout,
captured source, reference image and baseline native test remains unchanged.
Unknown indices fail with zero published draw/image counts. Existing replay
APIs keep the original fixed-function projection. Twenty-two total exports are
verified in both Android native libraries.

The one-button suite now runs **275 checks and66 visible patterns**:
engine18/NV50/GPU-core76/shader25/Vulkan106. The original83 native cases remain
first and their failures are retained. Twenty-three new native cases cover all
11 candidate texture replays; unit/uniform W, reversed winding, U/V/UV offsets
and unchanged-repeat controls; and two UV readbacks with the unchanged0.0001
diagnostic bound. Every color result still compares every byte with the original
independent oracle. UV bounds never affect color acceptance.

Original visible variants0..54 are unchanged. Variants55..65 display the candidate
counterparts of texture indices0..10 (candidate inset=61). All run automatically
in one batch and are exported/uploaded by the existing opt-in report workflow.
They do not replace the default live path. Overall allPassed remains false if
any original case fails, even when every candidate check passes.

Reports retain original failures alongside the candidate. The server analyzer
adds separate baseline/candidate inset outcomes, candidate pass counts and
completeness; missing candidate checks cannot produce allCandidateChecksPassed.
The same-thread automatic handoff was demonstrated by this follow-up from the
seed GitHub artifact. Its stored queue entry was consumed and is not re-added;
no manual chat attachment was needed for this investigation.

Two subsequent physical RMX5061 / MT6989 / Mali-G720 reports now confirm all23
candidate checks and all11 candidate visible variants, while retaining the
original inset failure (65/66 visible patterns overall). This confirms the
lowering for this authored, narrowly validated profile on this device/driver.
It does not yet integrate that policy into a general guest-draw backend, cover
indexed/depth/clipped/smooth-interpolation draws, or establish Switch-game
compatibility. Keep baseline failures visible. Any later runtime integration
must preserve fallback behavior for rejected profiles and pass the same strict
oracles. Details and evidence links are in
[physical follow-up validation](validation/android-normalized-projection.md).
