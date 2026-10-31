# Physical G720 NVMap backing-import checkpoint

User-supplied 0.13.0-dev report: schema 3/workload 13, engine suite 8, GPU-core
suite 2. SHA-256:
`a3de79a1550068bfbf2da394aeea28e0c9ebf53250b714651adbca3f6a7b3256`.
Raw JSON is kept only under ignored reports/physical-g720/.

All twenty checks pass: eleven original guest NROs, three direct GPU-core cases
and six native Vulkan microchecks. Every engine/core field matches freshly
executed 0.13 reference results except host architecture (aarch64 on the phone).
No group/case errors are reported. The new backing-import case matches 817
instructions/ten IPC/two copied handles/five A/four B/162 descriptor bytes,
known debug output and zero handles. Passing engine results also require the
memory oracles, zero remaining transfer objects and zero lent bytes. This closes
the backing-import and ABI 2 phone checkpoint, including the complete expanded
report passing through ARM64 Rust/JNI.

Device/driver match prior reports: realme RMX5061/MT6989, Android 16/API 36,
Mali-G720-Immortalis MC12, Vulkan 1.3.247, raw driver 184553472, hardware renderer.
Validation is disabled; zero reported errors is not a validation-layer run.
Native BC remains unsupported and the six host Vulkan cases pass the BC1
conversion/sampling/update fallback.

This proves the exact owned import workload and host microchecks. It does not
establish NV GPU mapping/submission, guest shaders/rendering, console performance
or commercial game support. There is no physical ADB connection; phone results
come from the supplied JSON.
