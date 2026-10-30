# Android scheduling checkpoint

Build: 0.5.0-dev, version code 5. APK SHA-256:
`189b74a06afd041519b3e07ca211d7e3899bf0550df85abddd82e156acc1e1a7`.
Implementation and limits are described in [scheduling.md](../scheduling.md).

## Verified

- 73 Rust workspace tests; formatting and Clippy with warnings denied.
- 16 independently LLVM-assembled fixtures, including five original NROs.
- Original engine cross-builds for ARM64/x86_64 using pinned Rust 1.85.1/NDK.
  Both final shared objects contain the Rust export, JNI entry and new workload.
- Android Gradle debug build/lint, signature, architecture and 16 KiB ELF/ZIP
  packaging checks pass. This is not a 16 KiB Android runtime test.
- Installed as an update over 0.4.0-dev on the isolated Android 15/API 35
  x86_64 AVD. Its workload-4 cached report is rejected and Export starts disabled.
- Ran the combined checks through the UI. All five actual guest programs and
  all six unchanged SwiftShader Vulkan workloads pass. The new guest's trace
  matches the host oracle: 97 instructions, two threads, four context switches,
  three blocking waits, one timeout and 1680 reference virtual ns. All guests
  exit with zero remaining handles; register/flag/TLS/argument/memory checks
  are performed by the Rust runner.
- Document-picker export matches the retained report byte for byte. Force-stop
  and cold launch restore the same current report, including the scheduling
  case, and enable Export without rerunning the checks.
- Captured logs show no app crash or app ANR. The emulator's System UI startup
  ANR blocked the first attempt to tap Run, so no new report arrived. Its
  XML-derived Wait action dismissed the system dialog; a fresh UI tree was
  used to start the actual successful run. Crash buffer is empty.

UI actions used current XML bounds, and the completed screenshot was inspected.
Only `emulator-5580` was used. Raw reports, screenshots, logs, native extracts
and APKs remain in ignored `reports/` and `artifacts/` directories.

## Physical evidence and remaining scope

The supplied [0.4.0-dev phone report](g720-engine.md) already passes the four
prior engine checks through the ARM64 boundary and all six G720 GPU checks.
That predates this scheduler. A new physical report is needed for the fifth
guest case; the software AVD does not establish its ARM64/device result.

The scheduler is single core and its clock is a deterministic reference model.
Cancellation, remaining synchronization, guest GPU services/commands/shaders,
presentation and commercial-game support remain open. This APK is a debug-signed
development diagnostic, not a production emulator release.
