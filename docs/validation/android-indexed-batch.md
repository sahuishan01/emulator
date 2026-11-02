# Indexed geometry validation (0.22.0-dev)

## Scope and physical baseline

The supplied [_21_ report](g720-texture-batch.md) confirms the prior 121 checks
and updated-texture presentation on the physical Mali-G720. The new indexed
code is a separate checkpoint. No physical ADB target was used for this build;
target-phone confirmation of the new cases is pending.

## Host and packaging

All 200 Rust tests pass offline, including signed-base/format/bounds, source
ownership/feedback, late/second-draw rollback, winding, unused inputs and
effect/capture/launch budgets. Formatting, Clippy with warnings denied and all
30 independent LLVM fixture comparisons pass. Previously tracked NRO binaries
remain byte-identical. Four existing SM53 listings still match NVIDIA nvdisasm
12.8.55 exactly; the corresponding modules pass Vulkan 1.1 spirv-val.

Fresh host groups pass: engine 17 (suite 14), NV 50 (suite 3), GPU core 20
(suite 4) and shader 25 (suite 3). Every previous named Rust record matches
0.21.1 exactly. Fresh desktop llvmpipe executes all 22 Vulkan cases with the
Khronos validation layer enabled, zero errors and an empty error field. All
previous 19 native records are identical. The three new native cases issue
actual indexed draws; all framebuffer bytes match separate pixel formulas.

Pinned Rust 1.85.1/NDK builds for arm64-v8a and x86_64, Gradle assembly and lint
pass. APK verification checks the signature, both ELF architectures, all 18 Rust
exports, 16KiB ELF/ZIP alignment, sole exported HomeActivity launcher, no requested
permissions and exclusion of QA documents/provider. Packaging alignment is not
evidence of a 16KiB-page Android runtime test. App/code/schema/workload/native are
0.22.0-dev/24/4/24/10. The final APK is 10,547,704 bytes, SHA-256
`1f87a4c3089cf4d09f6bcea29eaa3450243c01332a3062244b470d2a859c5eb6`.

## Android application, graphics and reports

The owned API35/x86_64 software AVD started after 98.3% CPU idle, nice10/two
cores/2048MiB requested (the emulator raises this to2560MiB), and boot completion
was verified. A genuine 0.21.1 to 0.22 update preserves all four catalog rows and
the byte-exact prior workload23 report. Only dedicated QA app data was then
cleared for independent import testing. An initial read-only upgrade probe used
the wrong table name; it was corrected to the actual `titles` schema before
installing the update.

All 19 library/import/runtime/settings flows and all 17 Catch/control/lifecycle
checks pass. The scene passes 14 checks across all six patterns, including byte
and short quads, signed base vertices and the visibly incomplete u32 inset.
Ten captures check 128 displayed source-pixel centers each (1280 samples), in
addition to native full-frame comparisons. Rotation/background run on the new
indexed triangle; foreground reopening, six rapid requests and repeated teardown
pass. Screenshots were visually inspected, including portrait/landscape indexed
scenes and the landscape imported-program view. Scene snapshots record verified
pixels, one completed/presented frame and no error.

All 134 Android checks pass and all four Rust groups match fresh host results
exactly except architecture. The visible summary, real picker export, cold
restore and recovery of all three AtomicFile caches are byte-exact. Both stopped
Catch and texture snapshots are attached to normal exports. The genuine prior
workload23 cache was restored only for invalidation QA after the independent
library reset; all new results were executed, not seeded. There were no app
fatal exceptions/signals/ANRs, empty crash buffers in all four QA flows and no
System UI Wait/retry exception. The owned AVD was stopped and its process awaited
successfully.
Raw evidence remains ignored under `reports/android-qa/v0.22.0-dev/` and
`reports/v0.22-*`.

## Interpretation

This verifies the [bounded indexed draw profile](../indexed-geometry-batch.md).
Depth/stencil, perspective/clipping, general guest display/HID, broader CPU and
shader coverage, tiled/compressed/mipped textures, audio, saves, JIT, commercial
games and practical console performance remain open. A passing software run
does not establish the new code on the phone.
