# Physical G720 color-clear checkpoint

Supplied 0.16.0-dev report: schema4/workload16; SHA-256
`5f479e1db7a257d939ae0f11da192a6859debfbc61e2a20fccbffdfd3d458a2c`.
All60 checks pass: engine14 (suite11), NV34 (suite2), GPU-core3 (suite2),
Vulkan9 (workload4). All fields in the Rust groups match the corresponding host
reference reports except hostArchitecture (aarch64 on the phone).

The color-clear NRO matches 5571 instructions/25 IPC/2 copied handles/18 A/11 B/
603 descriptor bytes and zero post-exit handles. All sixteen added protocol cases
pass, including masks, scissor, formats, padding, permissions, rollback, reply/TLS
aliases, class matching and retained owners. The three Vulkan clear replays pass
all512 bytes against their independent image expectations; the first consumes
actual bytes emitted by our interpreted guest NRO.

Device: realme RMX5061/MT6989, Android16/API36; Mali-G720-Immortalis MC12,
Vulkan1.3.247, raw driver184553472, hardware renderer. Native BC absent.
Validation is disabled; zero recorded validation errors is not a validation-layer
run. This is evidence for the exact original clear/replay subset only.

The new 0.17 shader translation batch awaits physical validation. Neither this
report nor host offscreen checks establish guest draws, display integration,
commercial game compatibility or console performance. Raw report remains ignored.
