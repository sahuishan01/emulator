# Physical G720 live homebrew checkpoint

User-supplied0.19.0-dev report, schema4/workload19, SHA256
`1682b6f9f7de121b4d76106685a7dad8572d8dcf988ded63b93d9d6f4b89a557`:
all103 checks pass (engine15/NV50/core3/shader19/Vulkan16). All four Rust report
groups match the host0.19 references exactly apart from architecture (aarch64).
The native live111-frame Vulkan case passes too.

The demo capture records578 completed/presented frames, score9/misses1, four
explicit frame allocations and an empty error after normal shutdown. It uses the
hardware Mali-G720-Immortalis MC12, not a software renderer. The user also
observed the paddle and reported that holding its buttons acted as a single tap.
That is the0.19 click-only UI limitation, addressed in0.19.1.

Device remains realmeRMX5061/MT6989/Android16 API36. This confirms the bounded
original homebrew/live presentation checkpoint on the target phone. Validation
layers are disabled; zero recorded errors is not a validation-layer run. It does
not establish general display/HID, commercial compatibility or console speed.
The raw report remains ignored; no game files, keys or emulator core were used.
