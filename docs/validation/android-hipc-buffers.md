# Android HIPC-buffered DMA checkpoint

0.10.0-dev (version code 10), APK SHA-256:
`ee570fd9ade1618f6deccca4e871baadbbf6f1ebd5a118375247f683bccaa9d9`.

Verified 120 Rust tests, formatting and Clippy with warnings denied; all 19
independent LLVM fixtures (11 raw A64/eight NROs); pinned ARM64/x86_64 Rust
builds; Gradle build/lint; signature/native identities and 16 KiB ELF/ZIP
packaging alignment. This is not a 16 KiB Android runtime test.

On the isolated Android 15/API 35 x86_64 AVD, upgrading from 0.9.0-dev invalidates
the workload 9 report and disables Export. Workload 10/schema 3 passes all
seventeen checks: eight guest NROs (engine suite 5), three direct GPU-core
checks (suite 2) and six unchanged SwiftShader Vulkan checks (workload 3).

The new guest-hipc-buffered-dma case matches 327 instructions/four successful
IPC calls/three send buffers/one receive buffer/80 logical descriptor bytes,
expected output and zero handles. It submits seven method writes/one launch/
eight copied bytes through normal A/B grants and checks results inside the
guest. The separate inline guest/direct GPU-core DMA cases still pass.

Document export is byte-exact. A requested force-stop/cold restart restores
identical JSON, the new guest case and an enabled Export button. The screenshot
was inspected and taps used fresh XML bounds. The System UI startup ANR required
its Wait action. One uiautomator attempt produced no file despite returning
success; a fresh dump recovered. QA now requires the dump success message and
valid XML, rather than trusting exit status or a stale path. Captured app logs
show no crash/ANR; the crash buffer is empty and exit-info records the requested
force-stop. Only emulator-5580 was used, then shut down. Raw reports, logs,
screenshots, APKs and QA helper scripts remain ignored.

The [0.9.0-dev phone report](g720-guest-dma.md) passes all sixteen prior ARM64/
G720 checks. The buffered case also passes in the
[0.10.0-dev physical report](g720-hipc-buffers.md). Only normal,
positive A/B descriptors are supported under the documented bounds. Standard
NV services/handle initialization, shaders/rendering, console timing/performance
and commercial gameplay remain open. This is a debug-signed development diagnostic.
