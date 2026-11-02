# Android application foundation (0.20.0-dev)

Final debug APK: 10,023,009 bytes, SHA-256
`2142421c4f14d5aa42d2ba6e60b726b8f22925bb4bad5509daffde400e4a21f9`.
Version code 21, report schema 4/workload 21; native workload 7 and the 103-case
diagnostic corpus are unchanged. The retained application ID/debug identity
permits updating the earlier lab installation. The launcher label is Emulator.

## Build and host evidence

The 194-test Rust workspace passes. The four new program-boundary tests cover
actual uploaded hello/IPC/thread/draw execution, malformed/incomplete/over-limit
uploads, exact Catch recognition, abort/reuse, faults, the 2M-instruction bound,
terminal session release and calling-thread isolation. The final focused tests,
format check and Clippy with warnings denied pass. All 28 independently assembled
LLVM fixtures match. There are no external Cargo dependencies or new Rust unsafe
blocks/raw-pointer arguments.

Pinned Rust 1.85.1/NDK ARM64 and x86_64 builds, Gradle assembly and lint pass.
APK signature, both ELF machine identities, all 16 Rust exports, 16 KiB ELF LOAD
and ZIP alignment verify. Manifest checks confirm the Emulator label, HomeActivity
as the only exported activity, no permission requests or test provider, disabled
backup/cleartext and no test NRO assets in the main APK. This is packaging evidence,
not a 16 KiB-page Android runtime test.

The current host archive was rebuilt and all four diagnostic reference groups
regenerated. All 16 desktop Vulkan cases pass on llvmpipe with the Khronos
validation layer enabled and zero validation errors.

## Main application flow

`tools/qa_production_app.py` passes all 19 checks on the owned API 35/x86_64
SwiftShader AVD using this exact final APK. A separate test-only DocumentsProvider
supplies our own NROs, real slow pipes and rejected inputs through Android's file
picker. It does not inject catalog entries or fabricated native success results.

Coverage includes the library launcher; landscape library/program controls;
cancelled import cleanup; an import surviving rotation; actual native validation;
duplicate, bad-header, module-header and oversize rejection; search selecting the
real file; actual hello output and normal exit; a visible unsupported-instruction
fault; removing the copied file/catalog entry; cold-start catalog/recents; running
the private copy after uninstalling the provider; persisted settings; settings
applied to Catch; and a recorded live Catch session. Player settings are restored
to their defaults at the end. Screenshots are visually inspected; final crash
buffers are empty and logcat has no app fatal signal/exception or app ANR.

Early attempts exposed harness assumptions: a provider grid item was offscreen,
a previous picker task remained on top, and Catch was below recently played
imports. The runner now uses list mode, fresh safe scroll bounds, an explicit
fresh Home task and search. These attempts remain under ignored evidence folders;
they are not counted as passing runs. A complete 17-check app pass preceded the
landscape layout change; the final 19-check run repeats the full import/runtime/
settings flow against the final APK.

Raw evidence is retained under ignored `reports/android-qa/v0.20.0-dev/`, with
host logs/references and APK verification under ignored `reports/v0.20-*`.

## Player and support regression

All 17 visible/control/lifecycle checks pass with `qa_homebrew.py --hold-controls`
against this APK: actual displayed green/red/black pixels, guest animation,
Pause/Resume, buttons, drag, reset, collision/score, GPU completion/presentation,
background stop, foreground recreation, repeated teardown, four bounded frame
allocations, held repeat, release before the edge, both clamps, leaving-button
cancellation and clearing held input across background. Default settings hide
the frame/GPU lines in the normal player. Final crash buffer is empty and there
are no app fatal/ANR entries.

The full Android support batch passes all 103 cases. All four Rust report groups
exactly match fresh host references except architecture. The summary is visible,
export through the real system picker is byte-exact, cold restore is byte-exact,
and both the diagnostics and demo caches recover byte-exactly from AtomicFile
backup files. The old workload-20 cache is rejected before checks are rerun. To
isolate that invalidation test after resetting the dedicated test library, the
authentic pre-reset workload-20 report was restored; it was never used as a new
passing result. One save-picker XML transition required a bounded fresh dump retry.
Final support crash buffer is empty with no app fatal/ANR entries. Screenshots were
inspected. The explicitly owned AVD was stopped and its process awaited successfully.

## Limits

The AVD validates Android flow and software Vulkan only. The supplied physical
[0.19.1 report](g720-held-controls.md) passes 103 cases and records 285 presented
frames, but does not test the new 0.20 app/import UI. No new physical phone session
was observed on the build host. No general libnx/MOD0, commercial content, general
homebrew graphics, normal HID/VI, audio, save states, JIT or practical game
performance is established. Backgrounding ends Catch and returning restarts it.
This is a private development prerelease, not production qualification.
