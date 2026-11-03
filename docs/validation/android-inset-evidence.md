# Inset failure evidence validation (0.27.2-dev)

The supplied physical 0.27.1 report reproduces the previous Mali failure:
239/240 diagnostics, 54/55 visible patterns, native `guest-projection-texture-3`
and visible variant42. All 71 native records match the previous physical report.
All four complete Rust groups match the host except architecture. Each successful
pattern has current app/run identity, verified pixels, completion and a surface
update. See [physical evidence](g720-projection.md). This release is an
investigation checkpoint; no Mali repair or commercial gameplay is claimed.

[Twelve controls and bounded attachment evidence](../inset-evidence.md) are added
without changing guest commands, shaders, CPU raster semantics or Rust code.
Both CTest targets pass: nine exact-comparison cases cover coordinate reporting,
full image bytes, the eight-sample bound, depth-bit format, incorrect sizes,
empty reads and invalid dimensions. The Vulkan integration test corrupts only
test references and verifies that color, depth and live-scene mismatches are
rejected, the full failing pixel data survives JSON, and no failed frame is
marked completed, presented or pixel-verified. This test entry point is excluded
from Android builds.

All 83 host Vulkan checks pass with Khronos validation enabled, zero errors and
an empty report error. The previous 71 outcomes/names/details are unchanged;
new comparison fields are additive. Both full float UV captures exactly match
the independent reference on the host. This does not establish Mali precision.
All original shader modules plus the new diagnostic fragment shader compile
and pass SPIR-V validation. No existing emulator core is used.

Pinned two-ABI Android assembly, lint and all 24 updater JVM tests pass. Signature,
the previous app's signing identity, all 21 existing Rust exports, ELF machine
types, 16 KiB ELF/ZIP alignment and strict manifest checks pass. Version identity
is app0.27.2-dev/code32/schema4/workload31/native16. The original Rust/native
value layouts and updater behavior are retained. No permissions or dependencies
are added. Packaging alignment is not a 16 KiB Android runtime test.

Final APK size: 12,108,984 bytes. SHA-256:
`aee6dd1a5e50d25d9fd4643c099c6e9643fd206795bfff2a1c71a7c1bf558fae`.
A fresh packaging pass removes unused incremental ZIP gaps. Its non-native
payloads are byte-identical to the earlier tested package; the native libraries
also contain the final eight-coordinate preview bound. The final APK receives
its own complete device run.

Final Android QA passes 11 workflow checks and a fresh 252-check/55-pattern run.
All 55 screenshots verify 7,040 displayed pixel centers against independent
formulas. All 83 native test records match the host exactly, including all 70
evidence records and both UV bit-pair arrays. All 55 visible snapshots preserve
their attachment comparisons. The 206,878-byte JSON matches the private cache
and published Downloads bytes exactly. MIME type, pending-row publication,
sharing without a duplicate export, cancellation without publication, library
and cache preservation, and cold Diagnostics restoration all pass. The original
automatic report stays immutable. Single-run mode omits the three unrelated
interruption/rotation restart repeats; those are not newly claimed here.

A first attempt returned to Home during late boot/display reconfiguration after
variant1, with no completed report and no target crash. Its runner was interrupted
and raw XML/log/cache evidence retained. The precise cause is unconfirmed.
The same assertions passed after the system settled. A complete pre-bound run
also passed before the coordinate-preview limit changed. That change followed
a synthesized report with every one of 185 attachment comparisons failing:
sixteen previews exceeded the existing 1 MiB bound; eight previews plus all full
images fit in 850,382 bytes. Final native tests and the entire device run were
repeated after that correction. No pixel comparison or acceptance rule changed.

Final crash buffers are empty, without target fatal exception, native fatal
signal or ANR. The inset image and restored summary were visually inspected.
The owned API35 AVD started after 97.99% CPU idle, nice10/two guest cores,
2048 MiB requested (raised to 2560 MiB by the emulator). Compact 720x1280/density280
was reset to native 1080x1920/density420. The AVD was explicitly stopped, its
process awaited with exit0, and ADB confirmed empty. No physical ADB target was
used. Ignored evidence is under reports/v0.27.2-* and
reports/android-qa/v0.27.2-dev/.

The same private in-app updater can obtain this prerelease after publication.
The next physical report must provide the actual mismatch and UV values before
changing interpolation/sampling behavior. General projection/clipping, broader
runtime/display and commercial games remain open.
