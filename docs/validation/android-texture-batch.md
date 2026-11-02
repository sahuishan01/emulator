# Guest textures on Android (0.21.0-dev)

Final debug APK: 14,184,481 bytes, SHA-256
`f1bf88bc999a7d355e2ba8d976f9c82da9224f6962fbdce97350bdfd96c9f082`.
Version code22, report schema4/workload22, native workload8. ARM64/x86_64,
retained application ID/signing identity, private development prerelease.

## Host and build evidence

All195 Rust tests pass, including the new texture value bridge, exact ABI layouts,
three independent image formulas and invalid-index publication rules. The
expanded portable corpus covers actual vertex varying/IPA/TEX semantics, missing
or malformed texture bindings, nonfinite coordinates, unsupported modifiers,
header maps,1920 header bit mutations and2304 shader-code bit mutations with
bounded owned translation/invocation results. Eight added GPU-core cases exercise
the three complete images, descriptor/mode/bounds rejection, late-command rollback,
target feedback through direct and aliased addresses, capture across unmap/reuse,
retained-state draws after source updates and an unmapped texture.

All29 LLVM assembly comparisons pass; `nv-draw.nro` remains byte-identical.
The four original texture/varying SASS listings match NVIDIA nvdisasm12.8.55 in
SM53 binary mode exactly. Our four emitted modules pass Khronos spirv-val for
Vulkan1.1. Formatting and Clippy with warnings denied pass. No external Cargo
dependency, existing emulator core, Rust unsafe block or raw-pointer ABI was added.

Fresh host reports pass engine16/NV50/core11/shader25. The previous15/50/3/19
records match0.20 exactly, including counts and results. Fresh llvmpipe executes
all19 Vulkan cases with the Khronos validation layer enabled, zero validation
errors and an empty error. All previous16 native records match exactly. Each new
sampled RGBA/BGRA/changed-texture result matches every framebuffer byte.

Pinned Rust1.85.1/NDK cross-builds, Gradle assembly and lint pass. APK verification
checks signature, both ELF architectures, all17 Rust exports,16KiB ELF LOAD/ZIP
alignment, HomeActivity as the only exported activity, no requested permissions
and no QA provider/documents in the main package. Packaging alignment does not
establish a16KiB-page Android runtime test.

## Android application and display

The owned API35/x86_64 SwiftShader AVD was started only after98.56% CPU idle,
nice10/two cores/2048MiB requested (the emulator raises this to2560MiB), and boot
completion was checked. An earlier startup was refused at41.46% idle. No physical
ADB target was used.

The actual0.20→0.21 install preserves every existing catalog row and byte-exact
prior diagnostic cache, and adds the protected texture-scene row without a schema
change. The dedicated test app data was then cleared for independent import QA;
user-device data was not cleared.

All19 library/import/runtime/settings checks pass, including real provider
cancellation/rotation, invalid/duplicate/module/size rejection, selected-file
execution/output/fault, removal/private-copy persistence, recents and preferences.
All17 existing visible Catch/held-control/lifecycle checks pass. No app crash or
ANR is recorded; final crash buffers are empty. Two picker transitions required
bounded fresh XML retries.

`qa_texture_scene.py` passes11 checks: library launch, actual displayed RGBA
sampling, BGRA color preservation, changed texture contents, pattern wrap, rapid
requests, landscape/portrait surface recreation, background close, foreground
reopen and repeated native/surface teardown. Each capture checks all128 displayed
source-pixel centers against separate integer coordinate/palette formulas, in
addition to the native full-frame comparison. The source aspect ratio and all
controls remain visible in landscape. Seven captures pass (896 sampled display
points). Native snapshots record one completed/presented frame, verified pixels,
zero validation errors and empty errors. Screenshots were visually inspected;
crash buffers remain empty and logs contain no app fatal exception/signal or ANR.

## Combined batch and lifetime

All121 Android cases pass: engine16/NV50/core11/shader25/Vulkan19. All four Rust
groups exactly match fresh host references except architecture. The visible
summary is correct. Real picker export, cold restore and recovery of all three
AtomicFile caches (diagnostics, Catch, texture scene) are byte-exact. Both stopped
scene/player snapshots are included in the ordinary export.

The authentic pre-upgrade workload21 report was restored only to test cache
invalidation after the dedicated library reset. New passing results were executed,
not seeded. Save/restore/backup UI transitions each required one bounded XML retry.
Final crash buffer is empty with no app fatal/ANR. The explicitly owned AVD was
stopped and its process awaited successfully. Raw evidence remains ignored under
`reports/android-qa/v0.21.0-dev/` and `reports/v0.21-*`.

## Interpretation

This establishes the [owned texture/varying profile](../texture-draw-batch.md),
application flow and software-driver rendering. It does not establish the new
code on the user's Mali GPU; the latest supplied physical report is0.19.1.
General imported-homebrew graphics, tiled/mipped/filtered/compressed textures,
normal HID/VI/display queues, broader CPU/shaders, audio, saves, JIT, commercial
games and practical console performance remain open. The visible scene is an
original reference workload, not a newly playable console title.
