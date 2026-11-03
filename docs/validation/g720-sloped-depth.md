# Physical Mali interpolated-depth checkpoint (0.25.0-dev)

The user-supplied `emulator-gpu-report-1793698307483-6e6a0b4e.json` confirms
all 172 diagnostics and 23 ordered visible patterns on realme RMX5061 / MT6989,
Android 16/API 36, physical Mali-G720-Immortalis MC12. The original file is
61,326 bytes, SHA-256
`6102d6a51c23b8e83a63b902343a6e2341916e5249d66f3505933896e7b87593`.
Ignored archive: `reports/physical-g720/v0.25.0-dev-report.json`.

App/schema/workload/native are 0.25.0-dev / 4 / 28 / 13. Engine 18, NV 50,
GPU core 40, shader 25 and Vulkan 39 all pass. All four Rust groups exactly
match the corresponding host references after removing only host architecture;
all 39 native test records also match exactly. Software-renderer status is false.
Physical validation layers were disabled; reported validation errors are zero,
which does not establish an enabled-validation run.

Batch `1793698307483-6e6a0b4e` contains exactly variants 0 through 22, with the
same run ID and current version. Each has native pixel verification, one
completed/presented frame, a consumer surface update, pass status and an empty
error. The stopped final scene is variant 22 with one verified frame and no
error. The automatic export names the matching app-owned Downloads/Emulator
file. The batch spans about 36.9 seconds; the root report follows about 1.7
seconds later.

This confirms the bounded forward/reverse sloped depth cases on the target GPU.
The attachment alone does not independently verify cache/download byte identity,
manual controls, sharing, rotation, general interpolation precision, larger depth
layouts, full guest display services or commercial-game compatibility.
