# Physical Mali projection report (0.27)

The supplied realme RMX5061 / MT6989 / Android16 API36 report records
239/240 diagnostics and54/55 visible patterns passing on physical
Mali-G720-Immortalis MC12. The projected inset texture (case3, variant42)
fails the exact Vulkan/reference pixel comparison in both native diagnostics
and presentation. Other native records and all four complete Rust groups
match the host exactly after removing only hostArchitecture.

Validation layers were disabled; validationErrors0 and native error empty.
The final stopped variant54 passes, but does not override the recorded failure.
The automatically exported report correctly retains allPassed=false.
No gameplay compatibility or complete projection checkpoint is claimed.

The inset case includes nearest-sampling coordinates exactly at texel boundaries;
interpolation precision is a hypothesis requiring actual mismatched pixel data.
The existing strict checks remain. The updater work does not fix this rendering
issue or turn its result into a pass.

Ignored raw report: reports/physical-g720/v0.27.0-dev-report.json.
Size95837bytes; SHA256
b102439185869df51b034301a5e73f001fa6e3f8c5428a14fd5a6fa98b812b91.
App0.27.0-dev/code30/schema4/workload30/native15.
