# Handoff

## Scope and working agreement

Original emulator from scratch, Switch 1 first, Android on Realme GT 7 Dream
Edition / reported realme RMX5061 / MT6989 / Mali-G720-Immortalis MC12. Pokémon
Legends: Z-A is a later target; no commercial game runs yet. Earlier Eden/Lemon
patching is superseded. Do not copy/link/import their cores. Historical upstream/
checkouts are ignored and excluded from builds.

The user requested autonomous progress: after verifying a checkpoint, continue
the next authorized issue without repeatedly requesting another go-ahead. Keep
unsupported paths explicit; do not fabricate successful device/service replies.

## Implemented through 2026-10-31

Eleven original Rust crates: memory, a64, loader, ipc, kernel, services, session,
texture, gpu, CLI and android-ffi. No external Cargo dependencies. Ten core
crates forbid unsafe; the FFI permits only the export attribute in its current
implementation, with no unsafe blocks or pointer arguments.

- Memory: bounded transactional mappings, reserved/committed regions, full-range
  R/W/X preflight, protect/unmap, shared backing IDs/slices, CPU aliases and
  capability-limited pin views. Protect/unmap no longer copy bytes. Mapped bytes
  count logical mappings; resident bytes count originating allocations until
  all CPU/GPU/queue views drop. Both caps apply. A fragment retains its original
  allocation. Scoped MemoryLease lending adds weak region links, restricted
  owner permissions/borrowed metadata and final-owner restoration; active loans
  block mapping edits. Creation rejects aliases and outstanding backing views.
  No payload copying/revocation. Page tables/paged allocation remain future. See
  docs/gpu-memory-submission.md and nv-initialization.md.
- A64 integer interpreter subset, ADR/ADRP, wide immediates, branches/load/store,
  guest-owned MRS TPIDRRO_EL0 and SVC. SIMD/FP/atomics/JIT remain open.
- Static uncompressed NRO0 v0 subset with RX/R/RW, BSS, guarded main stack and
  minimal entry configuration. No MOD0/linking/NSO/full libnx runtime.
- Kernel: 256 live handles, HIPC/CMIF v0 transactions with rollback, sender PID
  replacement, shared cloned sessions, manual-reset events, separate event
  capabilities, readable/writable ClearEvent, thread waits and cancellation.
  Minimal SM Initialize/GetService exposes emu:test and emu:gpu diagnostics.
  Services can prepare pinned memory effects; reply/handle validation precedes
  commit and rejects effects that alias TLS reply backing. Oversized reply/
  handle exhaustion cancels effects and candidate state; trace space is reserved.
  Normal HIPC A/B descriptors now grant READ inputs/WRITE outputs (four buffers,
  8 MiB logical bytes/4096 backing spans, positive lengths); all ranges preflight
  before service preparation, and B-to-TLS aliases fail. Default/legacy services
  reject unused buffers. SVC 0x15 now creates aligned transfer objects (positive
  4 KiB guest pages, <=16 MiB each, 32 live objects; owner NONE/READ/RW; TLS
  arena excluded). Up to four incoming copied process/transfer handles retain
  actual objects; current-process pseudo 0xffff8001 uses opaque owner identity.
  No caller handle is consumed. nvdrv command 3 adopts a checked NONE-permission
  arena on the service snapshot; final receiver close restores CPU rights.
  Static/exchange/non-normal buffers, incoming moves/other copied object types,
  domains/TIPC, full NV bootstrap and remote transfer maps remain open. NV Open,
  Close and three buffered unallocated NVMap metadata ioctls now use bounded
  per-client fds/per-device handles, isolated snapshots and prepared B writes.
  Rejected replies preserve IDs, object state and output. See nvmap-devices.md.
  Allocation/import, GPU address spaces and NV submission remain open.
- Scheduler: one host executor, up to 16 lifetime guest threads, core 0/default,
  priority/FIFO selection, 64-instruction quanta, separate registers/flags/TLS,
  sleep/yield, blocking events/joins, virtual deadlines, bounded Blocked outcome.
  One virtual ns per instruction is a reference model, not console frequency.
  Cancellation interrupts active waits or coalesces a deferred flag; polls,
  immediate results and argument errors retain it. SleepThread is not woken.
- Ten original LLVM NROs: hello (5 instructions), counter (384/5 IPC), clone
  (529/7 IPC), events (87), threads (97/2 threads/4 switches/3 waits/1 timeout/
  1680 virtual ns), cancellation (106/2 threads/4 switches/2 waits/2 interrupts/
  0 timeouts/1091 ns), gpu-dma (338 instructions/4 IPC/10 methods/2 launches/
  16 copied bytes), gpu-buffers (327 instructions/4 IPC/3 A/1 B/80 descriptor
  bytes/7 methods/1 launch/8 copied bytes). All exit with expected output/memory
  and zero handles. nv-initialize adds 317 instructions/4 IPC/2 copied handles,
  receiver retention/clone cleanup and restored source value 42. IPC traces
  include successful buffer/copy counts and bytes. nvmap adds 719 instructions/
  9 IPC/2 copies/4 A/3 B/98 descriptor bytes, cloned-session device lifetime,
  Create/SIZE/Free and result 4096 at image offset 0x3030.
- GPU: bounded 40-bit GPFIFO and four modern method-packet forms; owned GPU
  mappings importing granted MemoryViews; atomic range access; stable mapping
  IDs. Address-space-bound decode queues capture immutable command bytes and
  retain source backing across unmap/address reuse. Packets can cross entries
  within a batch. Failed admission publishes nothing; release is FIFO and
  queue-ID checked. release_front releases decoded work, not GPU completion.
  Original per-subchannel class binding and synchronous linear DMA methods now
  execute through execute_front; whole-batch preflight precedes memory/state
  mutation, dependent aliases observe earlier writes and success consumes once.
  Caps: 64 launches/4 MiB scratch and copied bytes/4096 backing spans per batch.
  Only class 0xB0B5 and launch 0x182 are supported; overlap and other flags fail.
  prepare_front consumes a queue into a pinned owned transaction; drop cancels,
  commit executes once/returns the updated queue. An original NRO now reaches
  this engine through emu:gpu, an emulator-owned one-shot CMIF diagnostic with
  fresh GPU class state per call. NV backing allocation/mapping/submission, shaders, fences
  and rendering remain open. emu:gpu command 2 now uses A commands/source/entries
  and B output with narrow grants; it cannot read B or write A. See
  docs/hipc-buffers.md, guest-gpu-bridge.md and gpu-dma-methods.md.
- Original Rust BC1 RGBA UNORM decoder and generated GPU corpus. Original C++
  Vulkan lab: six offscreen transfer/depth/compute-to-draw/BC1 conversion/sampling/
  update checks. Native shaders, no guest GPU execution. Allocations count Vulkan
  backing requests only, excluding pipeline/driver and process heap overhead.
- Android schema 3/workload 12: engine has ten actual guest NROs (suite 7),
  gpuCore has three direct ownership/DMA cases (suite 2), gpu has six unchanged
  native Vulkan cases (workload 3). Combined allPassed requires all three.
  Both Rust exports return initialized value-only 4104-byte reports (4096
  payload bytes), caught panics, no borrowed allocations. JSON is retained and
  exported via Android's document picker. Guest JSON is now 3800 bytes; watch
  the 4096-byte cap when adding cases/fields (C/Rust ABI changes must match).
  No network/storage permission.

## Evidence

- 143 Rust tests, formatting and Clippy with warnings denied pass. Tests cover
  malformed input, rollback, shared memory/alias visibility, pinned resident
  caps, GPU mappings, entry-spanning packets, immutable capture, queue ownership,
  atomic admission failure and ordered release, alongside prior guest behavior.
- All 21 independent LLVM fixtures match: 11 raw A64 plus ten NROs.
- ARM64/x86_64 Rust cross-build, Gradle build/lint, signature/ELF architecture and
  16 KiB ELF/ZIP packaging checks pass. Not a 16 KiB Android runtime test.
- GPU-core host reference cases pass: shared aliases survive protect/unmap/
  address reuse; two queued batches/four writes retain originals and end with
  zero originating resident bytes/pending work. Android 0.7.0-dev passes all
  fourteen checks, upgrade invalidation, byte-exact export and cold restoration.
  System UI needed its startup ANR Wait action; no app crash/ANR captured. See
  docs/validation/android-gpu-ownership.md. Android 0.8.0-dev also passes all
  fifteen checks, exact export, upgrade invalidation and cold restoration; the
  DMA case has 11 writes/2 launches/16 copied bytes/zero retained backing. See
  docs/validation/android-dma.md. Android 0.9.0-dev passes all sixteen cases,
  upgrade invalidation, exact export and cold restoration, with no captured app
  crash/ANR. The new guest has 338 instructions/4 IPC/zero handles. See
  docs/validation/android-guest-dma.md. Android 0.10.0-dev passes all seventeen
  checks, upgrade invalidation, exact export and cold restoration, with no app
  crash/ANR. See docs/validation/android-hipc-buffers.md. Android 0.11.0-dev
  passes all eighteen checks, exact export, upgrade invalidation and cold restore;
  the new NV case has 317 instructions/4 IPC/2 copied handles/zero remaining
  handles, transfer objects or lent bytes. See android-nv-initialization.md under
  docs/validation/. Its final APK includes the W3-width regression fix.
  Android 0.12.0-dev passes all nineteen checks, upgrade invalidation, exact
  export and cold restore. NVMap matches 719 instructions/9 IPC/2 copies/4 A/
  3 B/98 bytes/zero handles. System UI startup ANR needed XML-derived Wait; no
  app crash/ANR was captured. See docs/validation/android-nvmap.md.
- Prior Android AVD runs pass their current suites, upgrade invalidation, exact
  export and cold restore. System UI can show a startup ANR: dismiss only its
  Wait action using current XML, then re-dump before tapping. Never dismiss an
  app ANR as infrastructure. Details are under docs/validation/.
- Supplied physical reports through 0.10.0-dev pass all seventeen guest/GPU-core/
  host Vulkan cases on ARM64/G720. Buffered DMA matches 327 instructions/4 IPC/
  3 A/1 B/80 descriptor bytes/expected output/zero handles. Device and driver
  match. See docs/validation/g720-hipc-buffers.md and earlier records.
  Android 16/API 36, Vulkan 1.3.247, raw driver 184553472; validation
  disabled, native BC false. No physical phone is connected through ADB.
- Earlier physical BC1 conversion/sampling/update results do not establish guest
  graphics, sustained performance, shader stages or Z-A compatibility. Raw JSON,
  captures, native extracts, games/firmware/keys and APKs remain ignored.

## Next work

1. Verify suite 7 on the phone with 0.12.0-dev: guest-nvmap-device-ioctls expects
   719 instructions/9 IPC/2 copies/4 A/3 B/98 bytes/zero handles. The previous
   transfer-memory case also needs phone verification (317 instructions/4 IPC/
   2 copies). Prior seventeen checks passed physically through workload 10.
2. Implement NVMAP_ALLOC/backing import and lifecycle, then NV GPU mapping/
   submission with original command execution. Current Create/SIZE/Free operate
   on unallocated object metadata only. Aruid/CloneCurrentObjectEx/remote maps
   remain open. emu:gpu remains the diagnostic path for synchronous original DMA.
3. Implement original shader decode/IR/SPIR-V and renderer integration. M4's
   complete guest-program-to-image gate remains open. Presentation/audio/input,
   broader content/services and performance follow their actual dependencies.

## Repository and environment

Private GitHub repository: https://github.com/sahuishan01/emulator, main.
The 0.12.0-dev prerelease contains the verified debug APK, ZIP fallback, checksums,
packaging report and notes; older snapshots are retained. Source and artifacts
must match the recorded commit. These are prototypes, not production releases;
use agent-tasks notifications only. Use permanent /releases/download/<tag>/...
links plus the release page, never signed CDN redirects. An expired redirect
previously produced ERR_INVALID_RESPONSE. Private assets also return 404 to
unauthenticated browsers: verified with 0.9.0-dev; authenticated downloads match
checksums. Open the release in Chrome signed into the owner/access account.
A server download does not prove the phone's browser/network path.

Rust/Cargo, LLVM/Clang 19, llvmpipe, Vulkan/Khronos validation, glslangValidator
and spirv-val are available. Android SDK: ~/Android/Sdk. Pins: Rust 1.85.1,
NDK 28.2.13676358, Gradle 8.9, AGP 8.7.3, Kotlin 2.0.21, SDK 35/build-tools
34.0.0, CMake 3.22.1, Java target 17 (JDK 21 installed). The cross-build helper
sets RUSTC explicitly; do not change the global toolchain. Host Clippy may be
newer: identity allocation uses checked CAS compatible with both toolchains.

The QA AVD lives under ignored reports/android-qa/avd. CPU-gate its background
startup at >=80% idle; use only emulator-5580, await boot/command completion,
require a dump success message plus valid fresh XML (exit 0 can leave no file),
use XML-derived coordinates, capture/view screenshots/logs, and shut
it down/await its process after QA. Do not affect another emulator/device.
