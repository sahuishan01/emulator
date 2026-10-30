# Android cancellation and GPU framing checkpoint

Build 0.6.0-dev, version code 6. APK SHA-256:
`38504f28b365ec38d9a744c00c0c747f95ee4d69b1c5feed567b8591fd25f14c`.

- 84 Rust tests pass; formatting and Clippy with warnings denied pass.
- All 17 fixtures match independent LLVM assembly/link output: 11 raw A64
  programs plus six original NROs.
- Rust engine cross-builds for ARM64/x86_64. Gradle debug build and lint pass.
  Both native libraries contain the Rust/JNI exports and cancellation workload.
- APK signature, native ELF identities and 16 KiB ELF/ZIP alignment pass.
- Installed over 0.5.0-dev in the isolated Android 15/API 35 x86_64 AVD.
  The old workload-5 report is rejected and Export starts disabled.
- The emulator's System UI startup ANR was dismissed using its XML-derived
  Wait action. Fresh UI bounds were used for the successful Run action.
- All six actual guest programs and all six unchanged SwiftShader Vulkan
  workloads pass through Android/JNI. Engine suite 3/workload 6 contains the
  new cancellation case: 106 instructions, two threads, four switches, two
  blocking waits, two interruptions, zero expired blocked waits and 1091
  reference virtual ns. The guest/runner check preserved result indices,
  coalescing, event state, readable ClearEvent and zero live handles at exit.
- Document-picker export matches the retained report byte for byte. Cold
  launch restores the same report, including cancellation, and enables export.
- Captured logcat/crash/exit information shows no app crash or app ANR; the
  crash buffer is empty. The completed screenshot was inspected. Only the
  isolated emulator-5580 was used and it was shut down after QA.

The original [GPU framing parser](../gpu-command-framing.md) passes host tests
for four packet forms, GPFIFO addresses/lengths and malformed/unsupported/budget
cases. It is not linked to the Android runner or guest services/rendering.
The Android checks above do not establish GPU parser execution on the phone.

Raw reports/captures/native extracts/APKs remain in ignored directories.
The supplied [0.5.0-dev physical report](g720-scheduling.md) already passes the
five prior guest cases and all six GPU cases on ARM64/G720. The new sixth
cancellation case needs its own physical report. This is not a 16 KiB Android
runtime test or a console-timing/performance claim. Guest GPU memory/submission,
method execution, shaders, presentation and commercial gameplay remain open.
The APK is a debug-signed development diagnostic, not a production emulator.
