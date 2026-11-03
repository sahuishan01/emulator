# Supplied physical Mali report: 0.24.0-dev

The supplied `emulator-gpu-report-1793694655518-f5281ad9.json` is 59,221 bytes,
SHA-256 `486da02f1cd1a8abb5a505c0e3c0ca81c708912acf56e0ac3e850ecdf0656136`.
It reports realme RMX5061/MT6989, Android 16/API 36 and physical
Mali-G720-Immortalis MC12. App/schema/workload/native versions are
0.24.0-dev / 4 / 27 / 12.

All **168 diagnostics pass**: engine 18, NV 50, GPU core 38, shader 25 and
Vulkan 37. Each of the four Rust groups exactly matches the verified 0.24 host
baseline after removing only `hostArchitecture`. All 37 native test records also
exactly match the host baseline. This includes every color/depth readback case
for never, less, equal, less-or-equal, greater, not-equal, greater-or-equal,
always and disabled depth writes.

All **21 ordered patterns pass** in complete run `1793694655518-f5281ad9`.
Each has current-version/current-run identity, native pixel verification,
completion/presentation, a surface-update callback, physical rendering and an
empty error. The final stopped scene is variant 20, read-only depth, with one
verified frame. New depth patterns report seven frame allocations each. The
pattern batch finishes about 33.5 seconds after start; root capture is about
35.3 seconds after start.

Export metadata names the same supplied JSON in `Downloads/Emulator`. Receipt
confirms the report reached the user, but does not independently compare their
cache/media bytes or verify manual input, rotation, sharing, camera-visible
appearance or general game performance. Native validation layers were disabled;
zero reported validation errors are not an enabled physical validation run.
Native BC compression remains false and ASTC LDR is advertised true.

The raw report remains ignored under
`reports/physical-g720/v0.24.0-dev-report.json`. This confirms the
[bounded flat-Z depth batch](../depth-batch.md), including occlusion and depth
writes on the target Mali GPU. Interpolated depth in the subsequent batch still
requires a new physical report. Commercial games remain unsupported.
