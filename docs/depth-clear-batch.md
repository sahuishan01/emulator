# Depth clears and expanded test matrix (0.26)

The supplied [0.25 physical report](validation/g720-sloped-depth.md) confirms
172 diagnostics and 23 visible patterns on the target Mali GPU. This batch adds
original B197 depth-only clear execution and sixteen real Vulkan clear/draw
scenarios. The same **Test all & export JSON** button now runs **208 diagnostics
and 39 patterns**. App/code/schema/workload/native versions are
0.26.0-dev / 29 / 4 / 29 / 14; engine 18, NV 50, GPU core 60 (suite 8), shader 25,
Vulkan 55. All previous named diagnostic records remain unchanged.

## Implemented scope

`SET_Z_CLEAR_VALUE` (0xd90) supplies a finite, nonnegative Z value in [0,1].
`CLEAR_SURFACE` (0x19d0) with value 1 clears the selected ZF32 depth attachment.
The attachment retains the [bounded depth profile](depth-batch.md): one aligned
512-byte GOB, dimensions at most 16x8, one layer, no sparse/bounds/MSAA modes.
Shared clip, clear rectangle and scissor state select the intersection. Empty
or disjoint rectangles complete without memory access, including an unmapped
target. A full 16x8 footprint needs write permission only; partial or smaller
logical attachments require read/write access to preserve every untouched bit,
including padding. Clears do not require a color target, shaders, or initialized
depth-test/write/compare state. The target's read-only flag must be zero.

Prepared clears retain target backing and ordered effects without publishing
until commit. They share byte, span and launch budgets with existing operations.
Previous writes are projected before partial clears and subsequent depth draws.
Color/DMA destination overlap and shader/vertex feedback are rejected by backing
identity, including aliases and write-only clears. Late failures, cancellation,
unmap/address reuse and final release preserve the existing transactional rules.
The older Catch `graphics_frame` ABI explicitly rejects this new operation.

Negative zero, nonfinite/out-of-range values, stencil, mixed color/depth clear
flags and unknown flags fail explicitly. Larger layouts, combined clear masks,
general live depth/display and commercial games remain unsupported.

## One broad batch

The sixteen rendered cases cover full clears to zero/one, right-half and
scissored regions, empty and last-pixel clears, overlapping clears, clears
before/between/after draws, read-only drawing after a clear, NEVER/GREATER/LEQUAL
comparisons, and two clears separated by draws. Each checks every color byte
and D32 value against independent coordinate formulas. The generator builds
sixteen authored command pages from our own existing assets; these are not
sixteen new interpreted NRO programs. Reproduce them with
`python3 tools/generate_depth_clear_fixtures.py --check`.

Twenty appended portable correctness groups check the render corpus, five clear
values, nine rectangle forms, scissor intersection, empty unmapped clears,
read-only target rejection, six flag failures, six invalid float values,
seventeen missing fields, fourteen invalid attachment states, write-only full
clears, permissions/holes/truncated footprints, color and shader/vertex aliases,
late rollback, cancellation/unmap/reuse/final release, exact byte/launch/span
budgets, retained state, all eight comparisons at adjacent float boundaries,
and 128 repeated resource cycles. Twenty-one new Rust unit tests expose these
groups and ABI/error handling independently. The groups report their actual
verified-item counts in JSON (257 total); they are broader than twenty individual inputs.

## Native execution and ABI

The new 21st value export `emulator_depth_clear_replay_v1` returns a 52,112-byte,
alignment-4 value: existing 51,500-byte DepthReplay prefix, original 128 depth
words, count and four 24-byte clear records (rectangle, value, draw position).
Offsets 51,500 / 52,012 / 52,016 are checked in Rust and C++. Older ABI layouts
and exports are unchanged. Invalid indices return an explicit error.

Vulkan uploads the original depth contents, then executes actual
`vkCmdClearAttachments` calls at their captured positions between draws. It
does not upload the expected cleared depth as a substitute. A matching subpass
self-dependency and BY_REGION early/late depth read/write barriers order clears
and depth draws. Empty regions issue no forbidden zero-extent Vulkan clear.
Readback verifies all color and depth bytes after completion. The same path
drives the sixteen new Android patterns and native diagnostics.

See [build and Android evidence](validation/android-depth-clear.md). New physical
Mali confirmation is pending; host/software Android evidence is separate.
