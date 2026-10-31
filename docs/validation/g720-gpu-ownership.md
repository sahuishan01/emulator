# Supplied physical ARM64 GPU ownership result

The supplied 0.7.0-dev report (schema 3/workload 7) passes all fourteen checks:
six guest NROs, two GPU-core ownership cases and six host Vulkan cases. Report
SHA-256: `ca6c6dfbe201a5dff87575532df731da76724e77f150fefd5092a762908f8dd3`.
The ignored raw copy is `reports/physical-g720/v0.7.0-dev-report.json`.

Device identity remains realme RMX5061 / MT6989 / Android 16 API 36,
Mali-G720-Immortalis MC12 / Vulkan 1.3.247 / raw driver 184553472. Both Rust groups
report `aarch64`. GPU-core suite 1 passes shared backing/alias visibility and
queued-command snapshot/lifetime checks. The latter captures two batches/four
method writes; both cases finish with zero originating resident bytes. All errors
are empty. The six guest cases retain their exact instruction/lifetime counts.

This closes the physical checkpoint in [android-gpu-ownership.md](android-gpu-ownership.md).
It is user-supplied JSON, not an observed physical ADB session. Native Vulkan
validation is disabled and BC support is absent. It does not establish the newer
DMA execution path, NV-service submission, shaders, presentation, sustained
performance or commercial gameplay.
