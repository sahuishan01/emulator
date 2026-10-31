# Android NV device and NVMap metadata checkpoint

0.12.0-dev (version code 12), APK SHA-256:
`bd8a61757c288f5ea3827a90903aa5dea4ebbdbdcb02703e725463ffb59bc8e1`.

Verified 143 Rust tests, formatting/Clippy with warnings denied, 21 independent
LLVM fixtures (11 raw A64/ten NROs), pinned ARM64/x86_64 Rust builds, Gradle
build/lint, signature/native identities and 16 KiB ELF/ZIP packaging alignment.
This is not a 16 KiB Android runtime test. The host engine report is 3800 JSON
bytes plus its example's newline, within the fixed 4096-byte payload.

On the isolated Android 15/API 35 x86_64 AVD, schema 3/workload 12 passes all
nineteen checks: ten guest NROs (engine suite 7), three direct GPU-core cases
(suite 2) and six unchanged SwiftShader Vulkan cases. An in-place upgrade from
0.11.0-dev invalidates its workload 11 report and disables Export before rerun.

The new guest-nvmap-device-ioctls matches 719 instructions/nine IPC calls/two
copied handles/four A/three B/98 descriptor bytes/expected debug output/zero
handles. It initializes NV, opens `/dev/nvmap`, uses a cloned session after
closing the original, creates a 4 KiB unallocated object, queries SIZE, frees
it, closes the descriptor and final receiver, checks restored source value 42,
and exits. FFI also checks no remaining transfer objects or lent bytes. The
host tests verify failure rollback, descriptor/object quotas, stale identities,
invalid requests/rights and distinct A/B buffer behavior. See
[nvmap-devices.md](../nvmap-devices.md) for the exact partial ABI and limits.

Document export matches saved JSON byte for byte. A requested force-stop and
cold restart restore identical JSON, the new case and enabled Export. Fresh
validated XML supplies all taps; screenshots were inspected. System UI showed
a startup ANR after suite completion, requiring its XML-derived Wait action.
No app ANR was dismissed. Captured app logs show no crash/ANR; the crash buffer
is empty and app exit-info records the requested force-stop. Only emulator-5580
was used, then shut down and its process awaited. Raw JSON, captures, helper
scripts and release artifacts remain ignored.

The [0.10.0-dev physical report](g720-hipc-buffers.md) passes all seventeen prior
ARM64/G720 cases. Both the 0.11 transfer-memory case and this NVMap case await
phone evidence. This checkpoint records unallocated object metadata; it does
not import/allocate backing, map GPU memory, submit GPU work through NV, render
guest shaders or run commercial games. Full NV bootstrap, remote transfer
mapping, console timing/performance and 16 KiB Android runtime remain open.
This is a debug-signed development diagnostic.
