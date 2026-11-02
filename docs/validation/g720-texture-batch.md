# Supplied Mali-G720 texture report (0.21.1-dev)

The user's `emulator-gpu-report _21_.json` has SHA-256
`93aa554dd7879837d794219ffa3d6e2dd9e79b3cd3daac3260a4534aac7d6241`
(35,249 bytes). It reports RMX5061/MT6989, Android 16/API 36,
Mali-G720-Immortalis MC12, app 0.21.1-dev/schema 4/workload 23/native 9.
This is user-supplied physical evidence, not a build-host ADB session.

All 121 cases pass: engine 16, NV 50, GPU core 11, shader 25 and Vulkan 19. Every
record in all four Rust groups exactly matches the verified host references,
except `hostArchitecture`. The Vulkan device is physical, with an empty error.
Its validation layer is disabled; reported zero validation errors are not proof
of an enabled physical-device validation-layer run.

The texture-scene snapshot records variant 2 (the changed source texture), one
completed/presented frame, `pixelsVerified:true`, eight frame allocations, an
empty error and a normal stopped state. Its device is the same physical Mali.
The three sampled-texture offscreen Vulkan cases also pass in the GPU group.
The snapshot establishes presentation of the recorded variant; it does not by
itself establish touch timing or all window lifecycle paths.

This confirms the owned nearest/clamp/pitch-linear RGBA texture and screen-linear
varying checkpoint on the target phone. It does not establish general 3D models,
depth/indexed draws, broader texture/shader/runtime support, commercial games or
console performance. Raw copies remain ignored under `reports/physical-g720/`.
