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
  allocation; page tables/paged allocation remain future. See
  docs/gpu-memory-submission.md.
- A64 integer interpreter subset, ADR/ADRP, wide immediates, branches/load/store,
  guest-owned MRS TPIDRRO_EL0 and SVC. SIMD/FP/atomics/JIT remain open.
- Static uncompressed NRO0 v0 subset with RX/R/RW, BSS, guarded main stack and
  minimal entry configuration. No MOD0/linking/NSO/full libnx runtime.
- Kernel: 256 live handles, HIPC/CMIF v0 transactions with rollback, sender PID
  replacement, shared cloned sessions, manual-reset events, separate event
  capabilities, readable/writable ClearEvent, thread waits and cancellation.
  Minimal SM Initialize/GetService exposes only our emu:test counter. Incoming
  handles/descriptors/domains/TIPC and system device services remain unsupported.
- Scheduler: one host executor, up to 16 lifetime guest threads, core 0/default,
  priority/FIFO selection, 64-instruction quanta, separate registers/flags/TLS,
  sleep/yield, blocking events/joins, virtual deadlines, bounded Blocked outcome.
  One virtual ns per instruction is a reference model, not console frequency.
  Cancellation interrupts active waits or coalesces a deferred flag; polls,
  immediate results and argument errors retain it. SleepThread is not woken.
- Six original LLVM NROs: hello (5 instructions), counter (384/5 IPC), clone
  (529/7 IPC), events (87), threads (97/2 threads/4 switches/3 waits/1 timeout/
  1680 virtual ns), cancellation (106/2 threads/4 switches/2 waits/2 interrupts/
  0 timeouts/1091 ns). Each exits with expected output/memory and zero handles.
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
  No NV object/service submission, shaders, fences or guest rendering. See
  docs/gpu-dma-methods.md.
- Original Rust BC1 RGBA UNORM decoder and generated GPU corpus. Original C++
  Vulkan lab: six offscreen transfer/depth/compute-to-draw/BC1 conversion/sampling/
  update checks. Native shaders, no guest GPU execution. Allocations count Vulkan
  backing requests only, excluding pipeline/driver and process heap overhead.
- Android schema 3/workload 8: engine has six actual guest NROs (suite 3),
  gpuCore has three direct ownership/DMA cases (suite 2), gpu has six unchanged
  native Vulkan cases (workload 3). Combined allPassed requires all three.
  Both Rust exports return initialized value-only 4104-byte reports (4096
  payload bytes), caught panics, no borrowed allocations. JSON is retained and
  exported via Android's document picker. No network/storage permission.

## Evidence

- 102 Rust tests, formatting and Clippy with warnings denied pass. Tests cover
  malformed input, rollback, shared memory/alias visibility, pinned resident
  caps, GPU mappings, entry-spanning packets, immutable capture, queue ownership,
  atomic admission failure and ordered release, alongside prior guest behavior.
- All 17 independent LLVM fixtures match: 11 raw A64 plus six NROs.
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
  docs/validation/android-dma.md.
- Prior Android AVD runs pass their current suites, upgrade invalidation, exact
  export and cold restore. System UI can show a startup ANR: dismiss only its
  Wait action using current XML, then re-dump before tapping. Never dismiss an
  app ANR as infrastructure. Details are under docs/validation/.
- Supplied physical reports through 0.7.0-dev pass all fourteen guest/ownership/
  host Vulkan cases on ARM64/G720, with matching device/driver identity. The two
  ownership cases release all backing; queued capture counts are 2 batches/4
  writes. See docs/validation/g720-gpu-ownership.md and prior records.
  Android 16/API 36, Vulkan 1.3.247, raw driver 184553472; validation
  disabled, native BC false. No physical phone is connected through ADB.
- Earlier physical BC1 conversion/sampling/update results do not establish guest
  graphics, sustained performance, shader stages or Z-A compatibility. Raw JSON,
  captures, native extracts, games/firmware/keys and APKs remain ignored.

## Next work

1. Verify gpuCore suite 2 DMA on the phone with 0.8.0-dev: 1 batch/11 writes/
   2 dependent launches/16 copied bytes, zero retained backing/pending work.
   Prior guest/ownership/Vulkan checkpoints passed in the supplied phone reports.
2. Implement required NV service/ioctl/object registration so an original guest
   program can reach the DMA executor. Extend CPU/synchronization from owned
   workload needs. Queue capture and synchronous DMA are reference policies,
   not asynchronous fetch timing, a host Vulkan backend or guest completion.
3. Implement original shader decode/IR/SPIR-V and renderer integration. M4's
   complete guest-program-to-image gate remains open. Presentation/audio/input,
   broader content/services and performance follow their actual dependencies.

## Repository and environment

Private GitHub repository: https://github.com/sahuishan01/emulator, main.
The 0.8.0-dev prerelease contains the verified debug APK, ZIP fallback, checksums,
packaging report and notes; older snapshots are retained. Source and artifacts
must match the recorded commit. These are prototypes, not production releases;
use agent-tasks notifications only. Use permanent /releases/download/<tag>/...
links plus the release page, never signed CDN redirects. An expired redirect
previously produced ERR_INVALID_RESPONSE; a server download does not prove the
phone's browser/network path.

Rust/Cargo, LLVM/Clang 19, llvmpipe, Vulkan/Khronos validation, glslangValidator
and spirv-val are available. Android SDK: ~/Android/Sdk. Pins: Rust 1.85.1,
NDK 28.2.13676358, Gradle 8.9, AGP 8.7.3, Kotlin 2.0.21, SDK 35/build-tools
34.0.0, CMake 3.22.1, Java target 17 (JDK 21 installed). The cross-build helper
sets RUSTC explicitly; do not change the global toolchain. Host Clippy may be
newer: identity allocation uses checked CAS compatible with both toolchains.

The QA AVD lives under ignored reports/android-qa/avd. CPU-gate its background
startup at >=80% idle; use only emulator-5580, await boot/command completion,
use fresh XML-derived UI coordinates, capture/view screenshots/logs, and shut
it down/await its process after QA. Do not affect another emulator/device.
