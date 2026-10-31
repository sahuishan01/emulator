# Android linear DMA checkpoint

0.8.0-dev (version code 8), APK SHA-256:
`a584a059c05da44e5063e41718a4f30ef22c35644bfedea0e5fa6bfb607e691b`.

Verified 102 Rust tests, formatting and Clippy with warnings denied; all 17
independent LLVM fixtures; pinned ARM64/x86_64 Rust cross-builds; Gradle build/
lint; signature/native architecture and 16 KiB ELF/ZIP packaging alignment.
This does not establish a 16 KiB Android runtime.

On the isolated Android 15/API 35 x86_64 AVD, upgrading from 0.7.0-dev invalidates
the retained workload 7 report and disables Export. Workload 8/schema 3 then
passes all fifteen checks: six actual guest NROs, three direct GPU-core cases
(suite 2), six unchanged SwiftShader Vulkan cases (workload 3). The new original
DMA case captures one batch/11 method writes, executes two dependent launches,
copies 16 bytes through shared backing/aliases and retires all backing/pending
work. A consumed batch cannot execute twice. Data changes before execution are
visible; command changes after capture do not affect admitted work.

Document export matches the saved JSON byte for byte. A force-stop/cold restart
restores exactly that report, the new DMA case and an enabled Export button.
The completed screenshot was inspected; UI actions used fresh XML-derived bounds.
The emulator's System UI startup ANR required its Wait action. Captured app logs
show no crash/ANR, the crash buffer is empty and exit-info records only the
requested force-stop. Only emulator-5580 was used and it was shut down afterward.
Raw reports, screenshots, logs, APKs and QA helpers remain ignored.

The [0.7.0-dev phone report](g720-gpu-ownership.md) verifies prior ownership and
other cases on ARM64/G720. The direct DMA case also passes in the
[0.8.0-dev physical report](g720-dma.md).
The copy path is a synchronous CPU reference executor, independent of host
Vulkan; it is not an NV-service submission from an NRO or guest rendering.
Object registration/ioctls, shaders, presentation, console timing/performance
and commercial gameplay remain open. This is a debug-signed development diagnostic.
