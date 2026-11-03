# Supplied physical Mali report: 0.22.1-dev

User-supplied `emulator-gpu-report-1793635290111-6c1aaade.json` is 42,393 bytes,
SHA-256 `a3548a4989614f085eb591b0fe075537662de7adf9430aef00467325c504a815`.
It reports RMX5061/MT6989, realme, Android16/API36, app0.22.1-dev,
schema4/workload25/native10, physical Mali-G720-Immortalis MC12.

All 134 diagnostic cases pass: engine17/NV50/core20/shader25/Vulkan22. Every
record in the four Rust groups matches the verified host baseline exactly,
excluding hostArchitecture. All six ordered pattern snapshots belong to run
1793635290111-6c1aaade and have native pixelsVerified, completion/presentation1,
surfaceUpdated, no error and no software renderer. Indexed cases have ten frame
allocations, other cases eight. The final stopped scene records indexed variant5
with verified pixels and an empty error. No Catch snapshot is present.

The complete pattern sequence takes approximately 9.6 seconds; the root report
is captured about11.1 seconds after start. Export metadata names this same JSON
in Downloads/Emulator. Receiving the file confirms export reached the user;
this supplied JSON does not independently establish a byte comparison of their
on-device cache/media entry, touch behavior, rotation or sharing lifecycle.

The native validation layer is disabled. Reported zero errors therefore do not
establish an enabled physical validation-layer run. BC compression is unsupported
natively, while ASTC_LDR is supported. This confirms the bounded indexed/texture
checkpoint and automatic visible-pattern workflow, not general 3D/game support.
Raw copies remain ignored under reports/physical-g720/.
