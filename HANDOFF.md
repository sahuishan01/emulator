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

## Current implementation: 0.20.0-dev

Main app foundation: HomeActivity launcher/Emulator label, SQLite library with
included Catch, SHA-256 private-copy SAF imports, search/details/remove/recents,
actual imported static-NRO text execution and settings. Diagnostics moved under
Settings, all other activities nonexported; same app ID/debug signing identity.
See docs/android-app.md for contracts/limits. Program/Settings scroll; Home uses
full-page scrolling in landscape. No commercial game or general graphics support.

LibraryStore is application-owned, one serial worker, cancellation signal/stream
close,32MiB/file/256MiB/128imports, temp+orphan cleanup, no source-document deletion.
Jobs survive rotation, not process death. One blocked provider can delay its worker.
NativePrograms JNI uses new value-only ProgramChunk(4100/align4/off4) and six
program begin/append/inspect/start/step/close_v1 exports. Sessions are thread-local,
32MiB upload cap,1..4096-instruction slices/2M total,512-char output and explicit
fault/exit/wait/limit; close releases resources.16 Rust exports total.194 Rust tests.
App0.20.0-dev/code21/reportworkload21, schema4/nativeworkload7,103 checks unchanged.
ReportStore uses synchronized AtomicFile and bounded1MiB reads/backup recovery.

New QA: tools/qa_production_app.py drives real SAF through a separate test-only
tools/qa-documents module/generated owned fixtures; that provider is never shipped
with the main APK. tools/qa_homebrew.py uses Play Catch from library;
tools/qa_android_batch.py navigates Settings/Diagnostics and tests atomic backup
recovery. Packaging --main-app checks launcher/exported components/no permissions
and exclusion of test documents, in addition to all16exports/signature/16KiB layout.

The supplied _20_ physical report confirms0.19.1:103 passes/285presented frames,
score4/miss1/error empty. See docs/validation/g720-held-controls.md. New0.20
app/import flow still needs target-phone use; software AVD evidence is separate.

Validation:194 Rust tests/28 LLVM fixtures/fmt/Clippy, pinned two-ABI assembly/lint,
signature/all16exports/16KiB packaging/main-app manifest checks pass. Final APK
SHA2562142421c4f14d5aa42d2ba6e60b726b8f22925bb4bad5509daffde400e4a21f9
(10,023,009bytes). Fresh desktop16 Vulkan checks/zero validation errors; final
API35 AVD19 app checks/17 player-control checks/103 diagnostic checks and exact
host-group comparisons/export/cold restore/both atomic backup recoveries pass.
Screenshots inspected; empty final crash buffers; owned AVD stopped/awaited.
Evidence/initial harness failures/cache fixture provenance are recorded in
docs/validation/android-app-foundation.md. Canonical0.20 assets are emulator.apk
and emulator.zip (module/package IDs retain the older gpu-lab names).

Twelve original Rust crates, no external Cargo dependencies. Eleven core crates
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
  and content/game loading remain open. Fifteen owned NROs are LLVM-assembled.
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
  Bounded SPH/shader/draw reference execution now works; live host renderer
  integration remains open.
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
  All required state explicit; no tiled/depth/MRT/layer support in the clear subset; draw support is separate below.
  64 operations/4 MiB touched bytes/4096 spans are shared with DMA. Masked clears
  read current backing, full clears need only WRITE. Output/TLS aliases and
  failed replies cancel pixels/state/fences. See docs/color-clear-batch.md.
- Vulkan: six existing native offscreen transfer/depth/compute/BC1 conversion/
  sampling/update microchecks. Three new clear replays use original decoded operations, fixed host clear
  shaders, exact image oracles and RGBA/BGRA/masked targets. Replay 0 executes
  the new NRO and consumes its actual command bytes. Raw shader translation is separate below;
  no live renderer/display handoff. Rust BC1
  decoder/corpus remains independent. No console performance claim.

## Original raw shader batch (0.17 checkpoint)

Dependency-free `crates/shader` decodes raw SM53 32-byte scheduling/instruction
bundles into immutable typed SSA, evaluates serial reference semantics and emits
SPIR-V1.0 directly. Supports PT MOV32I/MOV/FADD/FMUL, vertex ALD input0x80/AST
position0x70 and EXIT/NOP only; fragment output is explicit R0..R3. No program
headers or guest draw integration. Undefined registers, missing outputs/exits,
predicates/modifiers/other interfaces/ops fail. Max2048 code bytes/2048 SPIR-V
words. Scheduling lower63 bits are nonsemantic hints for this register-only
subset; reserved bit63 fails. Normal finite arithmetic only established; bitmove
oracles also test special patterns. No external translator or emulator core.

Four owned SASS/raw fixtures match NVIDIA nvdisasm12.8.55 SM53 exactly, and all
emitted modules pass Khronos spirv-val Vulkan1.1. Public envytools hardware facts
were researched and independently disassembled; no implementation code copied.
15 portable cases/28 invocation oracles/117 rejections; all256 raw bytes have all
8 single-bit mutations tested (2048 mutants), plus every truncation. Actual
counts reported. See docs/shader-translation-batch.md for scope/commands/sources.

New emulator_run_shader_checks_v2 returns EngineReport ABI2; value-only
emulator_shader_draw_v1 returns17264 bytes/alignment4/modules+vertices+pixels.
Offsets16/8208/16400/16496/17008 asserted Rust/C++. Unknown index/panic fails with
zero word counts. APK verifier now verifies all6 Rust exports in both libraries.
Vulkan actually consumes emitted vertex/fragment SPIR-V in 3 added exact-pixel
checks: fullscreen green RGBA; FMUL-scaled red geometry with black border/BGRA
storage; green then scissored red using different shader pipelines in one job.
This older raw-code replay supplies layout/draw/scissor on the host. The new
guest-command path below derives those fields from actual commands.
Earlier clear replays still use fixed host shaders, as documented.

## Guest draw command batch (new)

Actual B197 region/pipeline/vertex attribute/stream/limit/viewport/scissor and
compact0x1214 non-indexed triangle commands now feed original SPH decoding,
shader SSA/SPIR-V and bounded CPU rasterization. Version3/SASS1 header profile:
VTG1 vertex/location0/position, PS2 pixel/target0; all other fields fail. Explicit
pipeline type/register allocation must match body use. GPU capture stops at EXIT
within2048 bytes. At most96 vertices,64x64 target, w1/no clipping, exact reversible
1/16 pixel grid, depth/stencil/blend/cull disabled/full RGBA mask. Top-left integer
coverage, uniform fragment output and transactional row effects. No general GPU
state/shaders/clip/interpolation/depth/indexing/instancing/async display claims.

Inputs/modules are owned snapshots; pins hold original backing. Output overlap
with any captured shader/vertex input, including GPU aliases, fails explicitly.
All64-op/4MiB effects+snapshot/module budgets and4096 pin-span limits shared with
DMA/clear. Methods preserve order; direct draw-then-DMA test observes new pixels.
Boxed draw registers deep-clone per request, no shared mutable snapshot metadata.

Original draw-rgba/bgra/scissor command+page fixtures:126words/4096bytes; full
black clear, full green draw, centered red draw. SPH/SASS offsets a00/b00/c00/e00,
vertices d00, target800, retained draw at400. New nv-draw.nro runs5571 instructions/
25IPC/2copies/18A/11B/603bytes; rejects128-word job, checks pixels/fence, closes
bound owners, redraws from retained state, closes channel/completion2 and restores
transfer marker42, zero resources. Every pixel independently checked. The older
clear NRO's equal instruction count is coincidental; separate fixtures/oracles.

16 new actual kernel/NV cases make50: correct pixels, BGRA/scissor, first-vertex
fetch, invalid headers/shader/registers/formats/ranges/domain/state, readonly draw
(clear disabled), late/reply rollback, B/TLS aliases, closed owners/clones, CPU
reuse, feedback, degenerate completion and shared budgets. Four SPH report cases
make19: positive vertex/pixel,1280 header-bit mutations, truncations/stage errors.
Shader totals282 instructions/2842 emitted words/30 invocations/1655 rejections.

New emulator_guest_draw_replay_v1 returns50452 bytes/alignment4: header16,
background60,3 DrawOp x16536, image512/error256. Offsets76/49684/50196 checked;
record modules/vertices56/8248/16440. Failure counts zero, owned bytes/no pointers.
Three Vulkan cases derive all shaders/vertices/target/scissor from these guest
plans (case0 runs actual NRO/page), compare full512 byte readbacks to independent
oracles/guest image. These are command replays after CPU completion, not live
renderer or display integration. Current replay fixture ABI supports6 vertices/
3 records separately from broader CPU limits. See docs/guest-draw-batch.md.

## Visible live homebrew (0.19)

Catch is an original interpreted NRO,64x64 ball/paddle with collision, score,
misses, reset and actual B197 submissions. Live GraphicsBackend is injected via
SM/NV factory. Kernel effects have a fallible complete() after reply/handle/alias
preflight, before CPU guest publication. Vulkan executes emitted SPIR-V and waits
real fences, compares full reference pixels; failure cancels guest bytes/state/
fences. Publication still uses GPU-verified CPU reference row effects, not general
GPU-authoritative memory. One full clear then1..3 draws/noDMA/same<=64surface/full
mask/<=6vertices is explicit live profile. ABI demo narrows to64x64/two rectangles.

Android Play homebrew demo opens TextureView with Vulkan FIFO swapchain/present
queue/per-image semaphores/nearest blit. One render worker owns native and TLS
Rust session. UI drag/buttons/D-pad input and frame pacing use an explicitly owned
mailbox at image a000, not Horizon HID/VI/BufferQueue. Background closes; resume
recreates; old textures retained until worker teardown. Pause, reset and visible
errors, atomic last-demo-report file included in normal exported report. Each
frame reclaims resources and command buffers; pipelines/readbacks per frame are
slow reference behavior. Full display services/coherence/async and games open.

Original shifted-register integer ADD/ADDS/SUB/SUBS supports W32/X64 LSL/LSR/ASR,
with independent LLVM vectors/flags/ZR/reserved forms. Session retains no mailbox
pin across initial transfer-memory loan (same data backing would conflict).
Clear IPC trace each frame to avoid the4096 diagnostic trace limit. Four-thousand
frames, failed backend/retry/reentry/input/order/reply-preflight tests included.
Three new value exports start(callback)/step/stop_v1, thread-affine, no Rust unsafe
blocks/pointers. LiveDraw66068bytes/pixels49684, DemoStatus296/error40;10exports.

Full suite now103 (engine15/NV50/core3/shader19/gpu16workload7), schema4/workload20 (0.19.1; native workload7 unchanged).
Prior102records remain unchanged. New GPU check runs111 live frames/catch/miss/
reset, no window. See docs/live-homebrew.md and Android validation for windowQA.

## Held buttons (0.19.1 patch)

The user found Left/Right only moved once per press. They now move immediately,
then repeat at80ms after240ms while held. Taps/accessibility click once; no extra
step on release. UI-thread held direction state cancels on UP/CANCEL/active
pointer up/leaving bounds and clears on Pause, drag, focus loss, background,
surface recreation and destroy. Opposite buttons neutralize. No native/Rust/
guest logic changed; both packaged native libraries are byte-identical to0.19.0.
App version0.19.1-dev/code20/reportworkload20;103 cases/nativeworkload7 unchanged.
The QA runner accepts --hold-controls for actual DOWN/UP holds, release/edge/
cancel/focus checks followed by the existing visible/control/lifecycle flow.
All17 pass on API35/x86_64. Native libraries byte-identical to0.19.0; pinned
build/lint/signature/all10exports/16KiB packaging pass. See
docs/validation/android-held-controls.md for final APK hash and evidence.
Fresh103 checks/exact references/export/cold restore pass; all screenshots
inspected, empty final crash buffers; owned AVD stopped/awaited. An earlier
batch lost ADB; restart/reinstall and authentic prior-cache fixture restoration
are recorded in validation notes.

## Batched Android report

Schema4/workload18: engine15(suite12), NV50(suite3, executesGuestCpu:false),
gpuCore3(suite2), shader19(suite2), Vulkan15(workload6), total102. All five groups
required; one run/export. Older workload caches are invalidated.

EngineReport ABI 2 remains 8200 bytes/alignment 4/offset 8/payload 8192. Its three
_v2 exports are unchanged. New emulator_guest_clear_replay_v1(index) is a
1264-byte/alignment-4 owned value: header, 8 ops x60 bytes, image512/error256.
Offsets/size are checked in Rust/C++; failures have zero operations/status!=0.
Host report sizes: engine5683/NV6557/core642/shader2891 bytes.

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

- 0.19 passes190 Rust tests/28 LLVM fixtures/demo generator/fmt/Clippy. Desktop
 16/16 Vulkan (111 live guest frames) validation enabled/errors0; prior15 GPU
 records and all four Rust groups unchanged. Pinned two-ABI APK builds/lints,
 signature/all10exports/16KiB packaging pass. See docs/validation/android-live-homebrew.md
 for final APK hash, visible QA and its limits. Visible controls/lifecycle pass,
 full103-case Android batch/export/cold restore pass; all screenshots inspected,
 no app crash/ANR; owned AVD stopped/awaited. Requested2048MiB was raised by this
 emulator to2560MiB. Supplied0.19 physical report passes all103 and records578 completed/presented
 hardware G720 frames/score9/misses1/empty error. All four Rust groups match
 exactly except architecture. See docs/validation/g720-live-homebrew.md.

- 183 Rust tests (including the thirty-four-scenario runner), fmt and Clippy with
  warnings denied; 26 LLVM fixtures (11 raw/15 NROs); pinned ARM64/x86_64 build,
  Gradle lint, signature and 16 KiB ELF/ZIP packaging checks pass. Packaging is
  not a 16 KiB Android runtime test. Keep exact APK hash in validation/release notes.
- Supplied 0.14 phone report passes all 21 prior cases, matching all engine/core
  fields against fresh reference output except host architecture. New mapping
  matches 3172 instructions/15 IPC/2 copies/9 A/6 B/372 bytes. See
  docs/validation/g720-nv-gpu-map.md. Device realme RMX5061/MT6989/Android 16/API 36/
  G720-Immortalis MC12/Vulkan1.3.247/raw driver184553472; validation off/native BC
  false. No physical ADB. The supplied 0.15 report now passes all 40 cases; see
  docs/validation/g720-nv-channel-batch.md. All three Rust groups match fresh
  references except architecture. Supplied 0.16 color-clear/replay now passes all60 checks on G720; see
  docs/validation/g720-color-clear-batch.md. All three Rust groups match the host
  references except architecture. Supplied0.17 now passes all78 checks, including three translated shader cases;
  all four Rust groups match references except architecture. See
  docs/validation/g720-shader-translation-batch.md. Supplied0.18 passes all102; four Rust groups exactly match fresh references
  apart from architecture. See docs/validation/g720-guest-draw-batch.md.
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
- Android0.17 passes all78 with four exact Rust reference comparisons, visible
  summary, upgrade invalidation and byte-exact export/cold restore. Both screenshots
  inspected; no app crash/ANR. Desktop12/12 with validation/zero errors. A System UI
  startup ANR Wait was handled via fresh XML; owned AVD shutdown/await succeeded.
  See docs/validation/android-shader-translation-batch.md for APK hash/evidence.
- Android0.18 passes all102 with four exact Rust reference comparisons, visible
  summary, upgrade invalidation and exact export/cold restore. Desktop15/15 with
  validation enabled/zero errors; all previous per-case records unchanged. Both
  screenshots inspected. System UI startup ANR Wait plus two empty picker XML
  retries recorded; no app crash/ANR. Owned AVD stopped/awaited. Final APK hash and
  limits in docs/validation/android-guest-draw-batch.md. All7 Rust exports verified
  in both libraries.183 Rust tests/26 LLVM fixtures and new draw fixtures pass.
- Historical software/physical evidence is under docs/validation/. Supplied
  microchecks do not establish guest graphics, full compatibility or performance.
  Keep reports, captures, helper outputs, games/keys/firmware/APKs ignored.

## Efficient local workflow

SDK ~/Android/Sdk; Rust1.85.1, NDK28.2.13676358, Gradle8.9/AGP8.7.3/Kotlin2.0.21,
SDK35/build-tools34.0.0/CMake3.22.1/JDK21 targetJava17. Cross-build helper pins
RUSTC and uses 2 jobs. Do not change global toolchains or touch other workspaces.
Desktop CMake now requires the owned host Rust archive; run cargo build --offline
-p emulator-android-ffi first. Gradle tracks examples/gpu/*.bin as inputs.
Generate host references via reference_report (default, gpu-core, nv, shader).
Gradle also tracks examples/shaders/*.bin.

Use tools/qa_android_batch.py for the entire upgrade/run/summary/export/restore/
log flow. It compares all four Rust groups to the host JSON, not just allPassed.
It verifies fresh XML before deriving taps, waits for asynchronous save navigation,
uses bounded retries/timeouts and can resume export/restore from an already
verified run. For0.18 pass --gpu-tests15 --gpu-workload6 --shader-reference reports/v0.18-host-shader.json. The runner can --shutdown its explicitly owned AVD. Initial and
resumed attempts/captures must be retained; do not hide app ANRs as infrastructure.

Owned AVD: reports/android-qa/avd, gpu-lab-api35, emulator-5580. Before background
startup sample /proc/stat and require >=80% idle; nice 10/cores 2/memory 2048,
headless SwiftShader/no snapshot. Await boot and dependent commands; do not run
concurrent UI operations. Stop only that AVD and await its process. System UI
startup ANR Wait may be handled using its exact current XML; app ANRs fail QA.

## Next combined work

Keep the visible Catch path functional while broadening guest graphics:
textured geometry, vertex/fragment interfaces, depth/indexed draws and the
required guest services. Implement normal display-buffer handoff/VI/BufferQueue
and HID to replace the diagnostic mailbox for broader programs. Grow CPU/runtime
and executable loading against real independent homebrew needs. Preserve the
reference backend, explicit unsupported-state errors and batched phone checks.
Optimize cached pipelines/resources and avoid unconditional readbacks only with
correctness evidence. General asynchronous GPU coherence/fences remain open.
Commercial startup/content/audio and Pokémon Z-A are not implemented.

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
