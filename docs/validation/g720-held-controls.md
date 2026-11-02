# Supplied G720 held-control report (0.19.1)

The user's `emulator-gpu-report _20_.json` has SHA-256
`cf18ecce40abe5fae7556de7f9a519a0d5c98de385775e4feee7dd5f36807dd0`.
It reports app 0.19.1-dev, schema 4/workload 20, RMX5061/MT6989 and
Mali-G720 Immortalis MC12. This is user-supplied physical evidence, not an ADB
session observed on the build host.

All 103 checks pass: engine 15, NV 50, GPU core 3, shader 19 and Vulkan 16.
The four Rust groups match the host references byte-for-structure except for
`hostArchitecture`. The stopped Catch session records frame 285, 285 completed
and presented frames, score 4, one miss, paddle X 12, four frame allocations and
an empty error. The user also confirmed the controls and asked to focus on the
main application.

The report confirms a successful run of the held-control build on this device.
It does not itself encode touch-down/up timing; the explicit hold/release cases
were tested on the owned Android AVD as described in [Android control QA](android-held-controls.md).
It does not validate the new 0.20 library/import UI, general homebrew compatibility,
commercial games, normal HID/VI or performance. Raw report copies remain ignored.
