# Physical G720 combined channel checkpoint

Supplied 0.15.0-dev report: schema 4/workload 15; SHA-256
`dda791574462194257cf9585c228ebe0994840ad0f5b1975207d082216580b38`.
All 40 checks pass: thirteen interpreted NROs (engine suite 10), eighteen
kernel/NV API scenarios (NV suite 1), three GPU-core cases and six native Vulkan
workloads. Every field in all three Rust groups matches freshly executed
0.15 reference reports except host architecture (aarch64 on the phone).

The channel guest matches 7179 instructions/32 IPC/2 copied handles/25 A/18 B/
875 descriptor bytes and zero post-exit handles. Kernel/NV cases record actual
IPC/verified-byte counts and zero post-exit handles. Vulkan reports validation
off, zero validation errors and a hardware renderer; this is not a validation-
enabled driver test.

Phone remains realme RMX5061/MT6989, Android 16/API 36,
Mali-G720-Immortalis MC12, Vulkan 1.3.247/raw driver 184553472. Native BC is false;
the existing conversion fallback still passes. No physical ADB target is connected.
The raw report is retained ignored. This confirms the prior synchronous channel/
DMA/fence/lifetime batch, not the later color-clear/replay batch or guest shaders,
rendering/presentation, commercial games or performance.
