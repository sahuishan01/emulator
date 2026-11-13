# Projection candidate validation — 0.27.5-dev

App/code35, schema4/workload32/native17,275 checks/66 patterns:
engine18/NV50/GPU-core76/shader25/Vulkan106. The original83 native checks and55
visible variants retain their inputs, oracles, shaders and acceptance rules.
This adds a candidate backend beside the original path; no physical fix is
claimed and the default path is not promoted.

The supplied physical0.27.3 report remains246/252 and54/55. Four actual inset
colors exactly match the colors predicted from captured actual UVs, which fall
slightly below reference texel boundaries. Variable W has60 differing UV pairs
and maximum4.097819328e-7 error; unit W has72 differing pairs and maximum
3.814697266e-6, yet passes strict color. This does not prove driver nonconformance.
The new post-guest XYZ/W policy is justified only for the current validated
positive-W, unclipped, screen-linear profile; see [contract](../normalized-projection.md).

270 workspace Rust tests pass, including two independent SPIR-V value-flow
tests and the new texture replay preservation test. An additional targeted
eligibility test passes, covering invalid W, off-grid, indexed and depth-enabled
rejection (271 distinct tests total). The SPIR-V test evaluates generated
instructions independently with21 inputs per shader across three actual guest
vertex programs, including guest arithmetic/transformed varyings. It checks
division after guest execution, unchanged varying bits and unchanged default
emission; fragment-stage lowering is rejected. The replay test checks all11
candidate cases against original inputs, fragment modules, samplers/textures,
draw metadata and independent images; invalid indices publish zero counts.

All22 emitted candidate vertex modules pass spirv-val for Vulkan1.1. Desktop
llvmpipe passes106/106 checks with Khronos validation enabled and zero errors.
The first83 records exactly equal the previous0.27.2 desktop records as parsed
JSON. All four fresh host reference reports are unchanged from0.27. Eighteen
guest/50NV/76GPU-core/25shader checks remain semantically unchanged. Both native
CTest checks pass, retaining strict failure-evidence self-tests. The initial
test command referenced two nonexistent fixture names; corrected to existing
owned shader fixtures before the full passing run, with initial output retained.

39 Android JVM tests pass. Fourteen Python collector/analyzer tests pass,
including new separation of original/candidate failures and incomplete candidate
rejection. No updater/upload implementation, permissions or dependency changes.
Appreport parsing remains compatible with complete failures and the new counts.
Assembly/lint/signature/twoABIs/22exports/16KiB ZIP+ELF alignment/strict manifest
pass. Optional new value export retains existing layouts and every old API.

An initial synthetic report with all219 attachment comparisons failing retains
full images/eight previews and fits within1MiB at1024898bytes. Do not increase
preview limits without repeating this export-bound check. Existing UV diagnostic
bound0.0001 is unchanged and never changes strict color/depth acceptance.

The automatic GitHub handoff produced this existing-thread follow-up. Server
state records the consumed queue entry, and repeated timer polls do not re-add
the same report SHA. No daemon restart or extra agent was used. The current
physical report remains archived outside source Git. APK install/physical runs
remain user actions; further phone evidence must arrive through opt-in uploads.

Ignored evidence: `reports/v0.27.5-*`,
`reports/android-qa/v0.27.5-dev/`, and `reports/github-inbox/`.

Android API35 SwiftShader: the complete batch passes275/275 and66/66, with all
8448 displayed pixel centers checked against independent formulas. All four
Rust groups match the fresh host references except host architecture. One
completed273299-byte JSON matches the private cache and published Downloads
bytes exactly; MIME/pending publication, sharing without another export, cancel
without publication, signed34->35 catalog/cache preservation and cold Diagnostics
restore pass. Single-run mode excludes interruption/rotation restarts; those
are historical, not newly claimed. All106 Vulkan results pass, validationErrors0.
The device analyzer recognizes all23 candidate checks separately and identifies
the GPU as software. No physical candidate result is inferred from this run.

The actual Android report synthesized with every218 attachment comparison failed
fits the unchanged1MiB export limit at1021367bytes. The larger initial synthetic
219-comparison template also fits. Candidate inset and cold-restored summary
screenshots were visually inspected. Startup System UI Wait was handled using
fresh XML and recorded; final crash buffer empty/no app fatal or ANR. Owned AVD
startup was declined while CPU idle was46.46%; successful startup at98.57%,
nice10/2cores/2048MiB requested (raised to2560). Existing720x1280/density280
retained, orientation restored, explicitly stopped/process awaited0, ADBempty.
No test package or synthetic token/backups remain.

Final APK12199284bytes/SHA256
9645d86f92ad2408781ccb7dc7db93329ce252e4362447456fc5d8b2dd9baf26.

## Physical follow-up — 0.27.5-dev and 0.27.6-dev

Two automatically collected reports from the Realme RMX5061 / MT6989 / Android16
device record the same result. The 0.27.5-dev report is 292,994 bytes with
SHA-256 `245277354ee6393a2f0b2b16ae4b8f6c1fac3ac75f0f2204d5e304e37c33e5b8`;
the 0.27.6-dev report is 292,994 bytes with SHA-256
`c76f1961cb8e93119ca1584dcbd40efadc7c67fab5040d201f7f079dcf07521d`. The
private artifacts are [issue 3, artifact 1](https://github.com/sahuishan01/emulator/issues/3/artifacts/1)
and [issue 4, artifact 1](https://github.com/sahuishan01/emulator/issues/4/artifacts/1).

Each physical run passes269/275 checks. Engine18, NV50, GPU-core76, and shader25
all pass; Vulkan100/106 pass. The same six strict failures remain: the baseline
projected texture replay and its five inset controls. The pattern batch is65/66;
only the original projected inset pattern (variant42) fails, with the same four
pixel mismatches. The observed colors still match the nearest texels predicted
from captured actual UVs. Validation layers were disabled, so zero reported
validation errors are not a validation-layer run.

The separate normalized-position candidate passes23/23 checks, including all11
authored texture replays, and all11 candidate visible variants pass on physical
Mali. The app-level result therefore remains `allPassed=false`; the candidate
does not mask or rewrite the baseline failure. The repeated0.27.6-dev report
shows this behavior persists in that build. This confirms the bounded unclipped,
positive-W, screen-linear authored profile on the reported device/driver. It is
not evidence that commercial Switch games render, nor evidence for indexed,
depth-enabled, clipped, or perspective smooth-interpolation draws. Promote it
only as a tested bounded policy when a real guest-draw backend consumes it; keep
strict baseline and rejected-profile coverage intact.

Raw JSON and analyzer files remain in ignored `reports/github-inbox/`; they are
not copied into source history.

## Repeated physical confirmation — issues #5–#15

The subsequent private issue reports cover0.27.6-dev through0.28.6-dev. Each
report hash was checked against its local JSON bytes, and the GitHub issue titles
confirm the version sequence. All eleven analyses produce the same result:
269/275 total checks,65/66 visible patterns,23/23 projection-candidate checks,
and four original inset pixel mismatches. The same six strict GPU checks fail;
the UV-predicted sampled colors still match actual output. The latest report is
[issue #15, artifact 1](https://github.com/sahuishan01/emulator/issues/15/artifacts/1)
for0.28.6-dev. These repeated runs strengthen confidence in the bounded
physical-device observation but do not broaden its validated draw profile or
establish commercial-game support. No baseline failure was relaxed.
