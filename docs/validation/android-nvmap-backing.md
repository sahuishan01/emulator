# Android NVMap backing-import checkpoint

0.13.0-dev (version code 13), APK SHA-256:
`8cdc2708976dbf8861f73f78f8ecfccc749769d94ca04ae86578add118c24397`.

Verified 151 Rust tests, formatting/Clippy with warnings denied, 22 independent
LLVM fixtures (11 raw A64/eleven NROs), pinned ARM64/x86_64 Rust builds, Gradle
build/lint, signature/native identities and 16 KiB ELF/ZIP packaging alignment.
This is not a 16 KiB Android runtime test. Both packaged native libraries export
the new `_v2` report functions and omit the incompatible ABI 1 names. The C/Rust
ABI 2 layout is 8200 bytes/alignment 4/offset 8, with 8192 payload bytes. Host
engine JSON is 4173 bytes, exceeding the old payload's capacity; all eleven
cases are present and passing, with initialized trailing bytes checked.

The isolated Android 15/API 35 x86_64 AVD passes all twenty checks in schema 3/
workload 13: eleven original guest NROs (engine suite 8), three GPU-core cases
(suite 2) and six unchanged SwiftShader Vulkan cases. In-place upgrade from
0.12.0-dev invalidates the workload 12 report and disables Export before rerun.

The new guest-nvmap-backing-import case matches 817 instructions/ten IPC/two
copied handles/five A/four B/162 descriptor bytes/known output/zero handles.
It initializes NV, opens/clones a device client, creates an object, imports
existing RW backing, checks Alloc's normalized alignment, preserves/updates CPU
bytes, queries ALIGNMENT and verifies Free's recorded address/size/flags before
closing resources. Expected memory: original transfer source 42 at image offset
0x2000, alignment 4096 at 0x3030 and updated import source 77 at 0x4000. FFI also
requires zero remaining transfer objects/lent bytes. The existing metadata guest
remains unchanged at 719 instructions/nine IPC/98 descriptor bytes.

Host unit/integration tests additionally prove actual imported-backing visibility,
read-only rights, retention across CPU unmap/address reuse, originating resident
charges until last release, client-wide byte/span limits, cloned receiver
lifetime and Alloc/Free/Close cancellation when replies fail. See the exact
[NVMap subset](../nvmap-devices.md); no new CPU payload allocation occurs.

Exported JSON matches private saved JSON byte for byte. Requested force-stop
and cold start restore identical JSON, the new case and enabled Export. Taps
use validated fresh XML; result/restored screenshots were inspected. System UI
needed its startup ANR Wait action before checks; no app ANR was dismissed.
Captured app logs show no crash/ANR, the crash buffer is empty, and exit-info
records the requested force-stop. Only emulator-5580 was used, then shut down
and its process awaited. Raw reports/captures/helpers/artifacts remain ignored.

The supplied [0.12 physical report](g720-nvmap.md) passes all nineteen prior
ARM64/G720 checks, closing transfer-memory and metadata guest phone checkpoints.
The [0.13 physical report](g720-nvmap-backing.md) now also confirms the
backing-import case and ABI 2 on ARM64/G720. Fresh
allocation, sharing IDs/cache control, NV GPU mapping/submission, guest shaders,
rendering and commercial games remain open. No console timing/performance or
16 KiB Android runtime validation is claimed. Debug-signed development diagnostic.
