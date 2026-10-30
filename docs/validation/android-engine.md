# Android original-engine checkpoint

Build: 0.4.0-dev, version code 4. APK SHA-256:
`e023690df3da8c3589b3d98104cbe3c494de1a1a00ff310bd585882969807f43`.
Original engine implementation and report ABI are described in
[android-engine.md](../android-engine.md).

## Verified

- 60 Rust workspace tests; formatting and Clippy with warnings denied.
- All 15 fixtures match independent LLVM 19 assembly/link output: 11 raw A64
  programs and four original NROs. No existing emulator implementation is used.
- Pinned Rust 1.85.1 engine cross-builds for ARM64 and x86_64. Both final shared
  objects export the Rust C function and corresponding JNI engine function.
- Gradle debug build and lint pass. APK signature, both ELF identities and
  16 KiB ELF/ZIP alignment pass the packaging verifier.
- Host report example actually executes all four NROs, checking output, guest
  memory, instruction/IPC counts and released process handles.
- Installed as an update over 0.3.0-dev in the isolated Android 15/API 35
  x86_64 AVD. Its retained workload-3 report is rejected; Export starts disabled
  and the UI asks for the updated checks.
- Ran **Run engine and GPU checks** through the UI. The schema-2/workload-4
  report has all four original-engine tests and all six Vulkan tests passing.
  Host architecture is x86_64, guest architecture AArch64, Vulkan renderer
  SwiftShader Device (Subzero). The GPU suite is unchanged workload version 3.
- Guest instruction counts are 5, 384, 529 and 87; IPC counts 0, 5, 7 and 0.
  Each program exits with zero live handles. Debug output and expected memory
  values are checked by the Rust runner, not supplied as fabricated results.
- Android document-picker export is byte-identical to the retained private
  report. Force-stop and cold launch restore the same report, show the guest
  results and enable Export without rerunning the checks.
- Captured logcat/crash buffer and process exit information. No app crash or
  app ANR was captured. The emulator's System UI showed a startup ANR, resolved
  with the XML-derived Wait action; this is an infrastructure limitation.

UI taps use current UI XML bounds, and the completed screen was inspected.
Only the isolated `emulator-5580` target was used. Reports, screenshots, logs,
native extracts and APKs remain under ignored `reports/` and `artifacts/`.

## Limits and next evidence

This does not execute the ARM64 library on hardware or test a 16 KiB Android
runtime. The new physical checkpoint is the same four guest programs through
the ARM64 Rust/C++/JNI boundary on the user's Realme/Mali-G720 phone. The
previous physical [six-workload GPU checkpoint](g720-sampling.md) remains valid
for that unchanged host corpus; it does not validate this new engine boundary.

Thread scheduling and blocking waits/timeouts, guest GPU commands/shaders,
Android guest-frame presentation and commercial games remain unimplemented.
The event suite covers manual-reset signaling and polling, not blocking waits.
This is a development diagnostic APK, not a production emulator release.
