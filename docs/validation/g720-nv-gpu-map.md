# Physical G720 NV GPU mapping checkpoint

User-supplied 0.14.0-dev report: schema 3/workload 14, engine suite 9/GPU-core
suite 2. SHA-256:
`67d82189959e6c7944f16a2c4f6c5510690ad2d2035f9be28d623b6455d826f1`.
All twenty-one cases pass. Every engine/core field matches freshly executed
0.14 reference results except host architecture (aarch64 on the phone). The
NV mapping guest matches 3172 instructions/15 IPC/2 copied handles/9 A/6 B/
372 descriptor bytes, known output and zero handles; passing also requires
its memory oracles and zero remaining transfer objects/lent bytes.

Device/driver match previous reports: realme RMX5061/MT6989, Android 16/API 36,
Mali-G720-Immortalis MC12/Vulkan 1.3.247/raw driver 184553472. Hardware renderer,
validation off/native BC false. This confirms owned mapping lifetime and the
six native Vulkan microchecks, not NV channel submission or guest rendering.
Raw JSON remains ignored. No physical ADB connection is available.
