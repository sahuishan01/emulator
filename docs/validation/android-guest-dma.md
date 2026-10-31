# Android guest DMA submission checkpoint

0.9.0-dev (version code 9), APK SHA-256:
`9177d31c648223edca1cbc6ad3d778997020cc880dac1ea26113a23183dba071`.

Verified 110 Rust tests, formatting and Clippy with warnings denied; 18 independent
LLVM fixtures (11 raw A64/seven NROs); pinned ARM64/x86_64 Rust builds; Gradle
build/lint; signature/native architecture and 16 KiB ELF/ZIP packaging alignment.
This is not a 16 KiB Android runtime test.

On the isolated Android 15/API 35 x86_64 AVD, upgrading from 0.8.0-dev invalidates
the workload 8 report and disables Export. Workload 9/schema 3 then passes all
sixteen cases: seven guest NROs (engine suite 4), three direct GPU-core checks
(suite 2) and six unchanged SwiftShader Vulkan cases (workload 3).

The new guest-gpu-dma-submission fixture executes 338 instructions/four IPC calls,
verifies both dependent copy results and reply counters inside the interpreted
guest, prints the expected message and exits with zero handles. Its emulator-owned
bridge submits ten method writes/two launches/16 copied bytes. The separate direct
GPU-core alias case retains its previous eleven writes/two launches/16 bytes and
zero remaining resident backing; these are distinct reference workloads.

Document export matches the saved JSON byte for byte. A requested force-stop and
cold restart restore exactly the same report, the new guest case and enabled
Export. The completed screenshot was inspected; all taps used fresh XML bounds.
System UI required its startup ANR Wait action; captured app logs show no crash/
ANR and an empty crash buffer. Exit-info records only the requested force-stop.
Only emulator-5580 was used, then shut down. Raw captures, reports, logs, APKs and
QA helper scripts remain ignored.

The [supplied 0.8.0-dev report](g720-dma.md) passes all fifteen prior ARM64/G720
cases. The guest-to-DMA case also passes in the
[0.9.0-dev physical report](g720-guest-dma.md). The bridge is
explicitly `emu:gpu`, not the nvdrv ABI; its copy engine runs synchronously on
the CPU. Standard NV services, guest shaders/rendering, console timing/performance
and commercial gameplay remain open. This is a debug-signed development diagnostic.
