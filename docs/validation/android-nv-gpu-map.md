# Android NV GPU address-space checkpoint

0.14.0-dev (version code 14), APK SHA-256:
`0b28baaacd3623f85b8c51425aebf375dfff6f447e0c92b72b9c31d24f47e623`.

Verified 161 Rust tests, formatting/Clippy with warnings denied, 23 independent
LLVM fixtures (11 raw A64/twelve NROs), pinned ARM64/x86_64 Rust builds, Gradle
build/lint, signature/native identities and 16 KiB ELF/ZIP packaging alignment.
This is not a 16 KiB Android runtime test. ABI 2 remains 8200 bytes with an 8192
payload; the host engine JSON is 4549 bytes and contains all twelve guest cases.

The isolated Android 15/API 35 x86_64 AVD passes all twenty-one cases in schema 3/
workload 14: twelve original guest NROs (engine suite 9), three GPU-core cases
(suite 2) and six unchanged SwiftShader Vulkan cases. In-place upgrade from
0.13.0-dev invalidates workload 13's report and disables Export before rerun.

The new guest-nv-gpu-address-space case matches 3172 instructions/fifteen IPC/
two copied handles/nine A/six B/372 descriptor bytes/known output/zero handles.
It imports NVMap backing, opens/initializes `/dev/nvhost-as-gpu`, maps a buffer,
checks the returned GPU VA/page size, clones the NV session, frees the handle
and closes its NVMap fd while mapped, checks the expected stale-handle error 4,
unmaps, closes resources and checks restored transfer memory and CPU bytes.
Expected memory: 4 GiB at image offset 0x3040, imported source 77 at 0x4000,
original marker 42 at 0x2000. FFI also requires zero transfer objects/lent bytes.
InitializeEx exercises a real input-only A/no B request; other ioctls use A/B.
Existing guest oracles are unchanged.

Host tests prove shared backing read/write visibility, readonly rights, partial
maps, CPU unmap/address reuse, resident charging until final release, client-wide
GPU byte/span quotas, fd ownership and non-reused client NVMap IDs. Failed Init,
Map, Unmap and Close replies preserve state/output/pins. Speculative GPU-space
snapshots preserve space identity and isolate tables; canceled mapping IDs are
never reused. See [the exact subset](../nv-gpu-address-space.md) and its limits.

Exported JSON matches saved JSON byte for byte. The first requested cold app
launch returned success, but the AVD exited cleanly with status 0 during the
following XML check; its cause is undetermined and final interval app logs were
unavailable. After CPU-gated AVD restart, the original report remained byte exact;
another force-stop/cold app launch restored the same JSON, new case and enabled
Export button. Fresh bounded XML verification passed and the restored screenshot
was inspected. The initial run needed System UI's startup ANR Wait action; no
app ANR was dismissed. Retry captures show no app crash/ANR and an empty crash
buffer. Only emulator-5580 was used, then explicitly shut down and awaited.
Raw reports, captures, helpers and artifacts remain ignored.

The supplied [0.13 physical report](g720-nvmap-backing.md) passes all twenty prior
ARM64/G720 cases, including backing imports and ABI 2. This new NV mapping guest
awaits phone evidence. Channel binding/submission, fixed/reserved/large-page
maps, sharing IDs/cache/fence interfaces, guest shaders/rendering and commercial
games remain unfinished. No console timing/performance validation is claimed.
This is a debug-signed development diagnostic.
