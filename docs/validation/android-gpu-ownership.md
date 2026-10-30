# Android GPU ownership checkpoint

0.7.0-dev (version code 7), APK SHA-256:
`854e225b5e1e299b3a46e167caacadc05e9f2ad85cf11e8087c983799735f42b`.

Final backing imports use fixed-length boxed storage, dropping spare Vec capacity
so retained payload accounting matches the owned allocation. Verified 94 Rust
tests, formatting/Clippy, all 17 independent LLVM fixtures,
both pinned Android Rust builds, Gradle build/lint, signature, native identities
and 16 KiB ELF/ZIP alignment. Both libraries include the new Rust/JNI entry.

On the isolated Android 15/API 35 x86_64 AVD, upgrade from 0.6.0-dev rejects the
old report and disables Export. After the System UI startup ANR's XML-derived
Wait action, all fourteen checks pass: six guest NROs, two actual GPU-core
memory/decode cases and six unchanged SwiftShader Vulkan cases. Schema 3/workload
7 requires all three groups. The new queue case captures two batches/four writes,
retains originals across address reuse and releases all resident backing/pending
work. Alias checks verify CPU/GPU shared writes, protection and unmap lifetimes.

Document export matches the saved JSON byte for byte. Cold restart restores
the GPU-core group and enables Export without rerunning checks. Captured logs
show no app crash/ANR and an empty crash buffer. The completed screenshot was
inspected; UI actions used fresh XML bounds. Only emulator-5580 was used and
shut down afterward. Raw reports, captures, native extracts and APKs stay ignored.

The [0.6.0-dev phone report](g720-cancellation.md) passes the six prior guest and
six GPU cases on ARM64/G720. The two new GPU-core cases need a phone report.
They are direct [memory/queue API checks](../gpu-memory-submission.md), not NV
service submissions or GPU execution. FIFO release does not signal completion.
Guest methods/shaders, presentation, 16 KiB Android runtime behavior and commercial
gameplay remain open. This is a debug-signed development diagnostic.
