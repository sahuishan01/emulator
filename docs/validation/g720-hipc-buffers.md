# Supplied physical ARM64 HIPC buffer result

The supplied 0.10.0-dev report (schema 3/workload 10) passes all seventeen checks:
eight guest NROs (suite 5), three direct GPU-core checks and six native Vulkan
checks. SHA-256: `e24bf3ed3e9fc4072b847427f4870f0ce7f08613e60843bb7bc2428af3a40c00`.
The ignored copy is `reports/physical-g720/v0.10.0-dev-report.json`.

The guest-hipc-buffered-dma case exactly matches 327 instructions, four IPC calls,
three send buffers, one receive buffer, 80 descriptor bytes, expected output and
zero handles. Both Rust groups report AArch64. Other cases also pass with empty
errors. Device/driver identity remains realme RMX5061 / MT6989 / Android 16 API 36 /
Mali-G720-Immortalis MC12 / Vulkan 1.3.247 / raw driver 184553472.

This closes [the buffered Android checkpoint](android-hipc-buffers.md). Evidence
is supplied JSON, not an observed physical ADB session. Validation is disabled
and native BC absent. These results do not establish the newer transfer-memory/
NV initialization case, NV device/ioctl support, shaders/rendering, sustained
performance, 16 KiB Android runtime behavior or commercial gameplay.
