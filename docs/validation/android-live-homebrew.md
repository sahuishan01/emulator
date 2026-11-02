# Android live homebrew checkpoint

Build0.19.0-dev: schema4/workload19, GPU workload7. Final APK SHA256
`24d4e566c1edba1218a5d4621fca30086aeadb0adefeb4f09791b9fd72a6e1f8`
(13,374,535bytes). Debug-signed ARM64/x86_64 prototype, private prerelease.
Signature, both intended ELF machines, all ten Rust exports, 16KiB ELF/ZIP
alignment, pinned cross-build and Gradle lint pass. Packaging alignment is not a
16KiB-page Android runtime test.

190 Rust tests pass, including >4,000 continuous guest frames, independent
rectangle pixels, host completion failure without guest publication, retry after
failure, reply preflight before callback, reentry, inputs and owned shutdown.
All28 LLVM fixtures and the new demo command/coordinate generator verify.
Fmt and Clippy warnings-as-errors pass. Desktop llvmpipe passes all16 Vulkan
checks with validation enabled and zero errors. The added check executes111
live interpreted guest frames, catches/misses/reset and checks exact64x64 pixels;
all prior15 per-case records remain unchanged. All four host Rust report groups
also remain byte/field identical to0.18.

The visible demo is exercised on the owned API35/x86_64 SwiftShader AVD. Evidence
is retained under ignored reports/android-qa/v0.19.0-dev/. The QA runner reads
fresh UI XML before every tap/drag and validates actual displayed red/green/black
pixels from a screenshot, separately from Vulkan's full readback oracle. It
checks pause, touch/button/D-pad input (including first press after touch), reset,
guest collision/score, equal completed/presented counts, background stop,
foreground recreation and repeated screen/session teardown. Four explicit frame
allocations remain constant; driver overhead is not included.

Earlier attempts are retained: the initial runner consumed an old XML when
uiautomator could not become idle amid status changes; it now requires a fresh
completed dump and status text updates less often. A screenshot helper initially
expected unavailable Pillow; it now reads Android's raw screenshot without extra
dependencies. Control tests exposed the first D-pad press being used to leave
touch mode/focus a button; the game view now holds focus in touch mode and key
handling precedes view navigation. Those failed attempts are not passing evidence.
Late TextureView destruction also releases directly if its already-stopped worker
has quit. No existing emulator implementation was used.

The first AVD startup shell failed its52.29% idle check but continued; the owned
process was stopped and awaited immediately. The corrected launcher exits before
launch on failure and started this AVD after97.28% idle, with nice10/two cores/
2048MiB requested/headless SwiftShader/no snapshot. The emulator automatically
raised its RAM to2560MiB; this was not a2GiB hard-limit run. Boot outlasted the first55-second poll;
boot completion was explicitly rechecked before installing/launching.

The supplied0.18 G720 report passes102/102; see
[g720-guest-draw-batch.md](g720-guest-draw-batch.md). New0.19 window/input behavior
awaits a physical report. Passing a small original homebrew does not establish
commercial game compatibility, full Horizon display/HID, async GPU coherence or
playable console performance. See [the live implementation limits](../live-homebrew.md).

Final visible QA passes all12 recorded flows in homebrew-pass/verification.json
with no retry notes. Screenshots visible-final.png and visible-catch.png were
visually inspected. The final APK also passes103/103 in batch/verification.json
(engine15/NV50/core3/shader19/Vulkan16), exact four-group reference comparisons,
workload18 cache invalidation, visible summary and byte-exact export/cold restore.
The exported JSON contains the stopped42-frame/presented42 demo capture. Both
batch screenshots were visually inspected. Both final crash buffers are empty;
no app fatal exception or ANR was captured. The owned AVD alone was stopped and
its process awaited (exit0); adb has no remaining devices.
