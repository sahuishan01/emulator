# Physical G720 NVMap and transfer-memory checkpoint

The user-supplied report captured by 0.12.0-dev has schema 3/workload 12,
engine suite 7 and GPU-core suite 2. SHA-256:
`4edd156ecc44f196750e128a0d254a2d448bdf6e659917cca45fe6cce2f8d590`.
Raw JSON is retained only under ignored reports/physical-g720/.

All nineteen checks pass: ten original guest NROs, three direct GPU-core cases
and six native Vulkan microchecks. Every engine/GPU-core field matches freshly
executed 0.12 host reference results, allowing only the host architecture difference
(aarch64 on the phone). There are no reported group/case errors.

The transfer-memory guest matches 317 instructions/four IPC/two copied handles,
expected restored-memory output and zero handles. The NVMap guest matches
719 instructions/nine IPC/two copies/four A/three B/98 descriptor bytes, expected
Create/SIZE/Free output and zero handles. Passing engine results also require
the memory oracles, zero remaining transfer objects and zero lent bytes. This
closes both new owned-program phone checkpoints from versions 0.11 and 0.12.

Device: realme RMX5061, MT6989, Android 16/API 36, Mali-G720-Immortalis MC12,
Vulkan 1.3.247, raw driver 184553472, hardware renderer. Validation is disabled;
zero reported validation errors is not a validation-layer run. Native BC remains
unsupported; the six host Vulkan cases include our BC1 conversion/sampling/update
fallback. Device/driver identity matches prior reports.

This evidence covers the exact owned programs and host GPU microchecks only.
It does not establish NVMap backing import/allocation, guest shader execution,
rendering, console performance or commercial game compatibility. There is no
physical ADB connection; the phone results come from the supplied JSON.
