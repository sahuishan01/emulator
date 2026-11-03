# Texture storage validation (0.23.0-dev)

The supplied [0.22.1 physical report](g720-one-tap.md) confirms all134 prior
diagnostics/six patterns on Mali-G720. New storage evidence below is from the
host and owned software Android device; new Mali confirmation is pending.

## Host and packaging

All205 Rust tests pass, including BC3 alpha/forced-four-color RGB, literal source
oracles, GOB offsets/uniqueness/padding/cropping, descriptor rejections, snapshots,
feedback/aliases, unmap/reuse, changed compressed alpha, late rollback and exact
decoded-memory budgets. Formatting and Clippy with warnings denied pass.
All31 LLVM fixtures verify independently; previous NRO binaries are unchanged.
The new storage generator reproduces all six source/page/RGBA datasets exactly.

Fresh host groups pass engine18/NV50/core30/shader25. Every previous named Rust
record is unchanged. Desktop llvmpipe runs all28 Vulkan cases with the validation
layer enabled, zero errors and an empty error field; all previous22 native records
are identical. New sources and every framebuffer color/alpha byte match separate
canonical fixtures and integer sampling/raster formulas.

Pinned two-ABI Rust1.85.1/NDK builds, Android assembly/lint, signature, arm64/x86_64
ELF, all19 exports,16KiB ELF/ZIP layout and strict main-app manifest checks pass.
Old18 value exports/layouts remain stable. App/code/schema/workload/native are
0.23.0-dev/26/4/26/11. Final APK is10,858,316 bytes, SHA-256
`32337b82818c51bf46c931cdee325049dc9ced3311966dae123e7b8d951e6eea`.

## Automatic Android workflow

The final APK passes14 one-tap flows, including two fresh complete151-check,
twelve-pattern runs. Every pattern belongs to its current run and has native
pixel/completion/presentation verification and a consumer surface update. Twelve
screen captures independently verify128 pixel centers each (1536 samples),
including all six new storage cases. Native readback checks alpha; opaque window
samples check RGB. Export bytes equal the private cache, MediaStore MIME/path/
published status are correct, and no picker is used.

Cancellation publishes no new file. Rotation/background restart unfinished work;
abandoned results cannot publish or join the new run. Completed rotation/share
preserve the file. Each completed invocation creates one distinct export. Cold
Diagnostics restores the complete twelve-pattern report exactly. Catalog/cache
are preserved by the actual upgrade and final same-version update.

Early screenshot attempts crossed pattern transitions. The harness discards such
captures, stores the exact verified raw frame as lossless PNG without a second
phone capture, and permits bounded fresh-run recapture of missing display points.
The final run needs no recapture and passes all14 flows. For capture speed the
owned AVD temporarily uses720×1280/density280, preserving its logical viewport;
the override is reset before manual tests. An intermediate BMP wrapper was
replaced by standard-library PNG encoding for compatible visual inspection.

## Manual controls and report recovery

At native1080×1920/density420, all20 manual scene/lifecycle checks pass. All12
patterns, rapid requests, source updates, landscape/portrait recreation,
background/foreground and repeated teardown pass. Sixteen captures check2048
independent display points. Screenshots of the multi-GOB and tiledBC3 patterns
were visually inspected. The automatic run was also visually inspected.

The standard runner resumes the actual new automatic151-check report. All four
Rust groups exactly match host references except architecture; all28 native
checks pass. Real picker export, cold restore and both present AtomicFile caches
(diagnostics/scene) recover byte-exact. The published automatic report remains
immutable after manual scene use. No new-version Catch snapshot was created,
so third-cache recovery is not claimed for this run. Two transitions required
bounded fresh-XML retries; there was no app ANR or System UI Wait.

All three final QA crash buffers are empty, with no app fatal exception, native
fatal signal or app ANR. The owned AVD was shut down and its process awaited.
Native-resolution settings were restored before shutdown. Raw evidence is
ignored under reports/android-qa/v0.23.0-dev and reports/v0.23-*; final automatic
evidence is in one-tap-final-apk/.

The AVD was started after98.3% CPU idle, nice10/two cores/2048MiB requested
(emulator raises this to2560MiB), and boot completion was verified. No physical
ADB target was used. Packaging alignment does not establish a16KiB-page runtime
test. This verifies the [bounded storage profile](../texture-storage-batch.md),
not general3D/depth/display services, commercial games or console performance.
