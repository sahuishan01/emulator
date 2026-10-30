# Supplied G720 converted-texture sampling result

The user supplied a GPU Lab 0.3.0-dev report, workload version 3, with all six
expected tests present and passing. `allPassed` is true and the error field is
empty. Device, driver and advertised feature identity match the earlier
0.2.0-dev report. No physical phone is attached to the development host; this
is user-supplied physical-device evidence. Raw JSON remains excluded from Git.

| Item | Reported result |
| --- | --- |
| Phone / GPU | realme RMX5061 / Mali-G720-Immortalis MC12 |
| Android / Vulkan | Android 16, API 36 / Vulkan 1.3.247 |
| Driver version, raw | 184553472, unchanged |
| App / workload version | 0.3.0-dev / 3 |
| RGBA8 transfer roundtrip | Passed |
| Depth-tested triangles | Passed |
| Compute-generated vertex visibility | Passed |
| BC1-to-RGBA8 conversion | Passed |
| Converted BC1 texture sampling | Passed, all rendered bytes match the CPU oracle |
| Texture update and resampling | Passed, updated pixels and both encoded sources match |
| Native BC compression feature | false, unchanged |
| Validation layer | Disabled |

The two new checks cover the [owned sampling/update corpus](../bc1-sampling.md):
compute conversion into an RGBA8 image, fragment sampling with known nearest
coordinates/clamped edges/transparent pixels, and reconversion/resampling of
the same decoded image in one submission. This records the intended path
working on the target Mali driver without native BC support. It closes this
physical sampling/update checkpoint; the unchanged workload need not be
repeated before further engine work.

## Allocation result

The report records 254688 requested Vulkan memory bytes across 19 allocations
for the entire lab. These are backing allocation requests, not process/GPU
working-set measurements. Driver/pipeline storage and Java/native heap usage
are excluded. Neither this number nor the advertised heap capacity establishes
a game memory budget or a conversion-cache policy.

## Remaining scope and next work

Validation was disabled, so zero validation errors is not a physical validation
layer pass. The corpus does not establish sustained performance, filtering,
sRGB, BC3/BC7, Tegra tiling, guest shader/command behavior, Android presentation
or commercial-game compatibility. It does not diagnose other emulators' Z-A
failures.

With this initial host feasibility path demonstrated, the next main-engine
slice is original kernel handles and IPC exercised by owned guest homebrew.
Scheduling and required ISA expansion follow that slice; guest GPU service
submission, commands and shader translation remain subsequent integration
work. Additional formats/tiling and resource-budget tests can progress
independently. These host results do not satisfy the guest graphics milestone.
