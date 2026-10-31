# Supplied physical ARM64 guest DMA submission result

The supplied 0.9.0-dev report (schema 3/workload 9) passes all sixteen cases:
seven guest NROs (suite 4), three GPU-core checks (suite 2) and six Vulkan checks.
SHA-256: `88f59b5eca70a8af9b8be8b97129af6133da7f4d12d0d80217bc081d4430b4b2`.
The ignored copy is `reports/physical-g720/v0.9.0-dev-report.json`.

The new guest-gpu-dma-submission case matches 338 instructions/four IPC calls,
expected verified-copy output and zero remaining handles. It runs on AArch64,
reaches the original DMA engine through emu:gpu and checks dependent results
inside the guest. Prior checks pass with empty errors; direct GPU-core DMA
remains one batch/11 writes/two launches/16 bytes/zero retained backing.

Device identity matches: realme RMX5061 / MT6989 / Android 16 API 36,
Mali-G720-Immortalis MC12 / Vulkan 1.3.247 / driver 184553472. This closes the
phone checkpoint in [android-guest-dma.md](android-guest-dma.md). It is supplied
JSON, not an observed ADB session. Vulkan validation is disabled and native BC
is absent. The copy engine executes synchronously on the CPU; these results do
not establish the newer HIPC-buffered case, standard NV compatibility, guest
shaders/rendering, sustained performance, 16 KiB runtime behavior or gameplay.
