# Supplied physical ARM64 linear DMA result

The supplied 0.8.0-dev report (schema 3/workload 8) passes all fifteen checks:
six original guest NROs, three GPU-core cases and six native Vulkan cases.
SHA-256: `0be279a234b0b64c6a5502537e0442f11e67d15411eec2dd5754999e8ae70b9f`.
The ignored copy is `reports/physical-g720/v0.8.0-dev-report.json`.

Both Rust groups report AArch64. The new `maxwell-dma-linear-copy` case reports
one captured batch, 11 writes, two launches, 16 copied bytes and zero resident
backing after release, with an empty error. Prior ownership/guest/Vulkan checks
also pass. Device identity is unchanged: realme RMX5061 / MT6989 / Android 16
API 36 / Mali-G720-Immortalis MC12 / Vulkan 1.3.247 / driver 184553472.

This closes the physical checkpoint in [android-dma.md](android-dma.md).
Evidence is user-supplied JSON, not an observed physical ADB session. Vulkan
validation is disabled and native BC remains absent. The DMA path is synchronous
CPU reference emulation; these results do not establish the new guest-to-DMA
service/NRO path, NV compatibility, shaders, rendering, sustained performance,
16 KiB Android runtime behavior or commercial gameplay.
