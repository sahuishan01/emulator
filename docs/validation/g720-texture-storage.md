# Supplied physical Mali report: 0.23.0-dev

The supplied `emulator-gpu-report-1793684305022-f88ee385.json` is 49,957 bytes,
SHA-256 `0555229a829d70cda57c75c7354dddcf1cb43e53436084d193bd9d9d1712eb87`.
It reports realme RMX5061, MT6989, Android 16/API 36 and the physical
Mali-G720-Immortalis MC12. App/schema/workload/native suite versions are
0.23.0-dev / 4 / 26 / 11.

All **151 diagnostics pass**: engine 18, NV 50, GPU core 30, shader 25 and
Vulkan 28. Every record in the four Rust groups exactly matches the verified
0.23 host baseline after removing only `hostArchitecture`. All 28 native test
records also exactly match the host baseline. Group counts and suite versions
were checked separately; confirmation does not rely only on `allPassed`.

All **twelve ordered patterns pass** in the complete run
`1793684305022-f88ee385`. Each snapshot has the current app version and run ID,
native pixel verification, one completed and presented frame, a consumer surface
update, no error and a physical renderer. The sequence includes tiled RGBA,
multi-GOB RGBA, pitched BC1, tiled BC1 transparency, pitched BC3 and tiled BC3
transparency. The stopped final scene is variant 11 with verified pixels and an
empty error. Native BC compression support is false, so these results confirm
the bounded software decode/upload fallback on the target Mali GPU. ASTC LDR
support is reported true; this does not test an ASTC decoding path.

The patterns finish about 19.1 seconds after start; the root report is captured
about 20.1 seconds after start. Export metadata names the supplied file in
`Downloads/Emulator`. The received JSON establishes that the report reached the
user. It does not independently compare their on-device cache/media bytes or
verify rotation, touch input, sharing, visual appearance or performance beyond
the reported native pixel and presentation checks.

Validation layers were disabled on the physical device. Zero reported validation
errors therefore do not establish a physical run with validation enabled. No
physical ADB session was used. The raw JSON remains ignored under
`reports/physical-g720/v0.23.0-dev-report.json`.

This confirms the [bounded texture storage profile](../texture-storage-batch.md)
and automatic report workflow. Guest depth, general 3D/runtime compatibility and
commercial games remain unsupported.
