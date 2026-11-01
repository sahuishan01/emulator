# Handoff

## Working agreement

Original Switch 1 emulator from scratch, targeting Android/Realme GT 7 Dream
Edition (reported RMX5061/MT6989/Immortalis-G720). Pokémon Legends: Z-A remains
a later target; no commercial game runs. Do not copy/link/import an existing
emulator core. Ignored upstream/ checkouts are historical and excluded from builds.

The user explicitly asked to test multiple things in one go because individual
APK/report cycles were taking too long. Batch connected features, successful and
rejected workloads, rollback and lifecycle coverage in one build. Continue larger
authorized steps without repeatedly asking go-ahead. Default suite runs all groups
in one tap and exports one report. Do not silently accept unsupported behavior.

## Current implementation: 0.16.0-dev

Eleven original Rust crates, no external Cargo dependencies. Ten core crates
forbid unsafe; android-ffi permits export attributes only, with no unsafe blocks
or pointer arguments. See README.md, docs/architecture.md, design.md and
implementation-plan.md for scope/milestones; M4 guest graphics gate is still open.

- Memory: bounded checked region mappings, stable shared backing IDs, CPU aliases,
  narrow MemoryViews, resident charging until the last pin drops, metadata-only
  protect/unmap, and MemoryLease lending. Active loans restrict source CPU rights;
  last lease drop restores them. Creation rejects aliases/outstanding backing
  views; no payload copy or revocation. Page tables/coherence remain incomplete.
- CPU/loader: original integer A64 subset with guest TLS/MRS/SVC, static NRO0
  RX/R/RW/BSS and guarded stack. FP/SIMD/atomics/JIT, NSO/MOD0/linking/full runtime
  and content/game loading remain open. Fourteen owned NROs are LLVM-assembled.
- Kernel: 256 handles, one host executor/16 lifetime guest threads, events,
  blocking waits, cancellation, virtual deadlines and cloned session identity.
  One virtual ns/instruction is a reference model, not console frequency.
  HIPC A/B: four each, 8 MiB logical/4096 spans; input READ/output WRITE. TLS
  reply aliases fail before effects. Service snapshots/handle/reply checks precede
  infallible prepared-effect publication. Bad replies preserve live bytes/state.
- Transfer memory: SVC 0x15 positive aligned 4 KiB ranges <=16 MiB, 32 live objects,
  owner NONE/READ/RW; W3 ignores high X3 bits; guest TLS arena excluded. Incoming
  copies support actual process/transfer objects only (max 4). nvdrv Initialize
  retains a checked process-owned NONE arena; final receiver drop restores rights.
  Full NV bootstrap/Aruid/CloneCurrentObjectEx and remote mappings remain open.
- GPU core: 40-bit GPFIFO/four packet forms, stable GPU mappings, captured command
  queues and original synchronous DMA (class B0B5/launch182), ordered copies,
  <=64 launches/4 MiB scratch/copied bytes per batch. Mapping snapshots preserve
  space identity; opaque host mapping IDs never reuse even for canceled branches.
  ChannelProcessor returns isolated successor state plus a cancelable effect.
  Guest draw/shader translation and host renderer integration remain open.
- NVMap/AS: 16 live fds/client; non-reused client-wide object IDs with owning-fd
  checks; 64 objects/owner. Alloc imports existing guest backing (READ/RW, linear,
  cached defaults), not fresh allocation. Import cap 64 MiB/4096 spans/client.
  AS uses automatic 4 KiB first fit from 4 GiB, 64 maps/space; GPU mapped cap 64 MiB/
  4096 spans/client separately. Free returns address 0 while a map retains backing.
  Fixed/reserved/large-page maps, sharing IDs/cache modes remain unsupported.
  InitializeEx public word 0/2 layouts disagree; both default flag 0/1 and big-page
  0/65536 combinations are accepted, other fields zero; actual maps remain 4 KiB.
- New channel batch: nvhost-gpu and nvhost-ctrl; SetNvmapFd, AS BindChannel, one
  DMA object context, bounded GPFIFO capacity (power of two <=256), inline SubmitGpfifo,
  SyncptRead and zero-timeout WaitEx. Registers persist per channel. Flags 2/3
  mean fence_get / already-satisfied wait+get; unsupported async modes fail.
  Each actual completed reference job advances a client-local point once. IDs
  never reuse, capped 32 lifetime channels; closed-channel values remain readable.
  This is synchronous CPU reference execution, not console hardware fence timing.
- Channels retain NVMap allocator and AS owners after external fd closes. Detached
  owners/maps stay charged until final referencing channel closes; open fds can
  also retain them. CPU unmap/address reuse cannot redirect commands or data.
  Combined GPU/B effects commit copies before reply/fence publication. B/DMA and
  active-TLS/DMA aliases fail atomically. No mutable metadata is shared between
  request snapshots. See docs/nv-channel-batch.md for exact operations/limits.
- Color clears: original B197 context, pitch-linear target 0, RGBA8/BGRA8 UNORM,
  finite colors, clear/scissor rectangles, channel masks and ordered row effects.
  All required state explicit; no tiled/depth/MRT/layer/draw/shader support.
  64 operations/4 MiB touched bytes/4096 spans are shared with DMA. Masked clears
  read current backing, full clears need only WRITE. Output/TLS aliases and
  failed replies cancel pixels/state/fences. See docs/color-clear-batch.md.
- Vulkan: six existing native offscreen transfer/depth/compute/BC1 conversion/
  sampling/update microchecks. Three new clear replays use original decoded operations, fixed host clear
  shaders, exact image oracles and RGBA/BGRA/masked targets. Replay 0 executes
  the new NRO and consumes its actual command bytes. No guest shader translator
  or live renderer/display handoff. Rust BC1
  decoder/corpus remains independent. No console performance claim.

## Batched Android report

Schema 4/workload 16: engine 14 interpreted NROs (suite 11), nv 34 kernel/NV
API scenarios (suite 2, executesGuestCpu:false), gpuCore 3 direct cases (suite 2),
gpu 9 Vulkan cases (workload 4). Run all checks executes all 60, summarizes actual
outcomes and exports one JSON. Combined allPassed requires every group. Older
workload caches are invalidated; missing groups show incomplete checks.

EngineReport ABI 2 remains 8200 bytes/alignment 4/offset 8/payload 8192. Its three
_v2 exports are unchanged. New emulator_guest_clear_replay_v1(index) is a
1264-byte/alignment-4 owned value: header, 8 ops x60 bytes, image512/error256.
Offsets/size are checked in Rust/C++; failures have zero operations/status!=0.
Host report sizes: engine5302/NV4427/core642 bytes.

New nv-color-clear.nro: 5571 instructions/25 IPC/2 copied handles/18 A/11 B/
603 descriptor bytes, full independent 128-pixel oracle at image5800, completion2
at4040, marker42 at3000 and zero handles/transfer objects/lent bytes. It rejects
partly planned clears, draws a three-region clear image, closes bound fds and
submits using retained state before final cleanup. Prior thirteen traces unchanged.
The sixteen added kernel/NV scenarios cover formats, masks, clip/scissor, padding,
rights, thirteen invalid states, late/reply rollback, B/TLS aliases, clones/closed
owners, class mismatch and empty clear completion. NV now restricts bind classes
by the allocated context (B0B5 or B197); standalone GPU tests allow explicit mixed
DMA/clear queues. GPU effects preserve order and pins through CPU address reuse.

## Evidence and verification

- 170 Rust tests (including the thirty-four-scenario runner), fmt and Clippy with
  warnings denied; 25 LLVM fixtures (11 raw/14 NROs); pinned ARM64/x86_64 build,
  Gradle lint, signature and 16 KiB ELF/ZIP packaging checks pass. Packaging is
  not a 16 KiB Android runtime test. Keep exact APK hash in validation/release notes.
- Supplied 0.14 phone report passes all 21 prior cases, matching all engine/core
  fields against fresh reference output except host architecture. New mapping
  matches 3172 instructions/15 IPC/2 copies/9 A/6 B/372 bytes. See
  docs/validation/g720-nv-gpu-map.md. Device realme RMX5061/MT6989/Android 16/API 36/
  G720-Immortalis MC12/Vulkan1.3.247/raw driver184553472; validation off/native BC
  false. No physical ADB. The supplied 0.15 report now passes all 40 cases; see
  docs/validation/g720-nv-channel-batch.md. All three Rust groups match fresh
  references except architecture. New 0.16 color-clear/replay awaits the phone.
- Android 0.15 passes all 40 cases with derived summary, upgrade invalidation,
  exact export and cold restore. A save-navigation timing issue in the QA harness
  was fixed with bounded expected-screen polling; resumed verification after AVD
  restart passed. Initial and resumed captures are retained; no app crash/ANR
  captured. See docs/validation/android-nv-channel-batch.md.
- Android 0.16 passes all 60 checks in one run, matching all three Rust references,
  with visible summary, upgrade invalidation and byte-exact export/cold restore.
  Desktop llvmpipe passes all nine with validation enabled/zero errors. API35 AVD
  needed a System UI startup ANR Wait (XML-derived); two empty picker dumps were
  retried successfully. No app crash/ANR; both screenshots inspected; owned AVD
  shut down and awaited. See docs/validation/android-color-clear-batch.md.
- Historical software/physical evidence is under docs/validation/. Supplied
  microchecks do not establish guest graphics, full compatibility or performance.
  Keep reports, captures, helper outputs, games/keys/firmware/APKs ignored.

## Efficient local workflow

SDK ~/Android/Sdk; Rust1.85.1, NDK28.2.13676358, Gradle8.9/AGP8.7.3/Kotlin2.0.21,
SDK35/build-tools34.0.0/CMake3.22.1/JDK21 targetJava17. Cross-build helper pins
RUSTC and uses 2 jobs. Do not change global toolchains or touch other workspaces.
Desktop CMake now requires the owned host Rust archive; run cargo build --offline
-p emulator-android-ffi first. Gradle tracks examples/gpu/*.bin as inputs.
Generate host references via reference_report (default, gpu-core, nv) example.

Use tools/qa_android_batch.py for the entire upgrade/run/summary/export/restore/
log flow. It compares all three Rust groups to the host JSON, not just allPassed.
It verifies fresh XML before deriving taps, waits for asynchronous save navigation,
uses bounded retries/timeouts and can resume export/restore from an already
verified run. For 0.16 pass --gpu-tests 9 --gpu-workload 4. The runner can --shutdown its explicitly owned AVD. Initial and
resumed attempts/captures must be retained; do not hide app ANRs as infrastructure.

Owned AVD: reports/android-qa/avd, gpu-lab-api35, emulator-5580. Before background
startup sample /proc/stat and require >=80% idle; nice 10/cores 2/memory 2048,
headless SwiftShader/no snapshot. Await boot and dependent commands; do not run
concurrent UI operations. Stop only that AVD and await its process. System UI
startup ANR Wait may be handled using its exact current XML; app ANRs fail QA.

## Next combined work

Continue the M4 graphics/shader/renderer slice with a known owned workload.
The color-clear replay is partial progress: guest shader decode/IR/SPIR-V, actual
draws and live rendering/presentation are still required. Batch needed command/state/shader and negative/lifetime tests together;
do not require a phone report per small opcode/ioctl. Broader asynchronous GPU
scheduling, error/event/fence contracts, CPU ISA/runtime/content, audio/input and
performance remain explicit work. No gameplay compatibility is claimed yet.

## Repository and delivery

Private repo sahuishan01/emulator, canonical main checkout here. Push verified
source, create a target-commit development prerelease (--latest=false), and upload
six assets: APK, ZIP containing APK/README/checksum/packaging report, APK checksum,
ZIP checksum, README and packaging report. Download them all back and byte/hash
compare; verify tag/main/source and private visibility. Give permanent APK/ZIP
/releases/download/<tag>/... links plus release page. Never send signed CDN URLs.
Private links return 404 when unsigned; use Chrome signed into an access account.

These are debug prototypes, not production releases: use agent-tasks notifications
via the protected ntfy-agent-notify helper. Do not use agent-releases. Never print
or copy credentials. Send the authorized task notification at final completion.
