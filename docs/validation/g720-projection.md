# Physical Mali projection report (0.27)

The supplied realme RMX5061 / MT6989 / Android16 API36 report records
239/240 diagnostics and 54/55 visible patterns passing on physical
Mali-G720-Immortalis MC12. The projected inset texture (case 3, variant 42)
fails the exact Vulkan/reference pixel comparison in both native diagnostics
and presentation. Other native records and all four complete Rust groups
match the host exactly after removing only hostArchitecture.

Validation layers were disabled; validationErrors0 and native error empty.
The final stopped variant 54 passes, but does not override the recorded failure.
The automatically exported report correctly retains allPassed=false.
No gameplay compatibility or complete projection checkpoint is claimed.

The inset case includes nearest-sampling coordinates exactly at texel boundaries;
interpolation precision is a hypothesis requiring actual mismatched pixel data.
The existing strict checks remain. The updater work does not fix this rendering
issue or turn its result into a pass.

Ignored raw report: reports/physical-g720/v0.27.0-dev-report.json.
Size 95837bytes; SHA256
b102439185869df51b034301a5e73f001fa6e3f8c5428a14fd5a6fa98b812b91.
App 0.27.0-dev/code 30/schema 4/workload 30/native 15.

The next supplied physical report, app0.27.1-dev/code 31 with the same workload,
reproduces exactly the same failed diagnostic and variant 42. All71 native test
records are identical to the previous physical report. The four complete Rust
groups still match host references, and all 54 successful ordered patterns carry
the current app/run identity, exact pixel/completion and surface-update proofs.
The final successful stopped variant 54 again does not override this failure.
Ignored raw report: reports/physical-g720/v0.27.1-dev-report.json, 95837bytes,
SHA256 106c50f41cf627689ba84438ccf9437277745936f4e5cca82b0a1cc6c463fcfc.

The 0.27.2 build adds [bounded failure evidence and twelve controls](../inset-evidence.md).
This is an investigation checkpoint, not a confirmed Mali rendering repair.
