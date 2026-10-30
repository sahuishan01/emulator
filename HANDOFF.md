# Handoff

## Scope and target

The user wants a new emulator written from scratch. The earlier Eden/Lemon
patching approach is superseded. Do not fork, link, import or copy their cores.
Untouched `upstream/` references remain ignored and outside all builds.

Target Switch 1 first, then other consoles. Primary phone: Realme GT 7 Dream
Edition / Dimensity 9400e / Immortalis-G720. Pokémon Legends: Z-A is a later
compatibility target. The earlier black screen in other emulators is not a
root-cause diagnosis for our code. No commercial game runs in this implementation.

## Implemented through 2026-10-31

- Ten original Rust workspace crates: memory, a64, loader, ipc, kernel, services,
  session, texture, CLI and android-ffi. No external Cargo dependencies. Nine
  core crates forbid unsafe code; android-ffi permits the C export attribute
  only and contains no unsafe blocks or raw-pointer arguments.
- Bounded memory with transactional batch mappings, reservation/commit,
  protect/unmap and cross-region accesses. Failed writes do not partially
  mutate memory. Protection/unmapping currently copy backing data; shared
  backing/alias and page models remain future work.
- A64 integer interpreter subset, including ADR/ADRP, wide-immediate forms and
  guest-owned MRS TPIDRRO_EL0.
  Unsupported instructions and services return explicit faults.
- Bounded static uncompressed NRO0 v0 subset, RX/R/RW segments, BSS, guarded
  stack and minimal homebrew entry configuration. MOD0/linking, NSO and full
  homebrew runtimes are unsupported. See docs/nro-loader.md.
- Original 256-entry handle table, HIPC/CMIF v0 and transactional service
  dispatch. Minimal SM Initialize/GetService exposes only our `emu:test`
  diagnostic counter. CMIF control 2 clones a shared session identity; closing
  the original retains state through its alias. Incoming handles, descriptors,
  domains, contexts and TIPC remain unsupported. No Nintendo device service
  returns fabricated success. See docs/kernel-ipc.md.
- Original single-core scheduler: CreateThread/StartThread/ExitThread/SleepThread,
  per-thread registers/flags/TLS, priority selection, FIFO equal-priority order
  and 64-instruction quanta. Up to 16 lifetime threads; core 0/default only.
  Reference time is one virtual ns per instruction, with direct idle deadline
  jumps. This is a deterministic model, not measured hardware frequency.
- Manual-reset events, blocking event/thread waits, joins and virtual timeouts.
  Validate full handle arrays, retain objects across handle closes/array writes,
  wake all existing manual-reset waiters, preserve X1 on timeout/error. With no
  runnable thread or future deadline, return bounded Session/RunOutcome::Blocked.
  Cancellation, mutexes/arbitration and negative yield variants remain open.
  See docs/scheduling.md.
- OutputDebugString and ExitProcess with bounded raw debug records and owned
  process TLS. Some guest Result errors are implemented; complete Horizon
  error-result/exception behavior remains unsupported. Exit releases handles.
- Session execution budgets pause and resume without repeating debug output.
  Our own hello.nro prints its known message and exits after five instructions.
  Three further original NROs test counter=42 (384 instructions/5 IPC calls),
  cloned counter=43 after closing its original (529/7), and event lifecycle
  (87/0). A fifth NRO checks two-thread events/joins/timeouts (97 instructions,
  four switches, three blocking waits, one timeout, 1680 reference ns). Its
  child/main registers, flags, TLS, arguments and final memory are verified.
  The raw CPU demo still produces 42 after 24 instructions.
- Original C++ Vulkan GPU Lab shared by desktop CLI and Android JNI. Six
  offscreen workloads: exact image-transfer roundtrip, overlapping depth-tested
  triangles, compute-generated vertices compared to the CPU-uploaded draw,
  BC1-to-RGBA8 compute conversion, converted texture sampling and image updates
  compared to our Rust reference and CPU texel-coordinate oracle. Both new
  dispatch/draw pairs run in one submission and retain both encoded inputs.
- Original bounded BC1 RGBA UNORM reference decoder, six tests and generated
  GPU corpus. Does not implement other BC formats, sRGB, guest tiling or sampling.
- GPU reports contain device/driver identity, physical feature advertisements,
  limits, heaps, queues, 11 format queries and per-workload results. A format
  query is not proof of texture sampling/decoding correctness.
- Allocation reports sum requested Vulkan memory across all workloads held
  until teardown, excluding driver/pipeline and host heap overhead.
- Kotlin Android lab with background native checks, retained report and
  user-selected JSON export. ARM64 and x86_64 APK now embeds the original Rust
  engine and executes five original NROs through JNI. Schema 2/workload 5,
  engine suite 2, adds per-case scheduling counters. It
  separates `engine` and `gpu` outcomes. The six GPU checks are unchanged.
  Rust returns a bounded initialized 4104-byte value-only C report with caught
  panics. No gameplay/session/surface API yet. See docs/android-engine.md.

## Verified

- `cargo test --offline --workspace`: 73 tests passed, including malformed IPC,
  identity replacement, reply rollback/permissions, atomic handle exhaustion,
  clone/event lifetimes, scheduling, joins, timeout progress while a thread runs,
  multi-waiter wakes, creation rollback/limits, deadline overflow and budget
  independence at 1/7/63/64/65 instructions.
- All 16 fixture binaries match LLVM 19 assembly/link output, independently of
  our decoder: 11 raw A64 programs plus five original homebrew NROs.
- Rust formatting and Clippy with `-D warnings`: passed.
- GLSL generation and spirv-val checks: passed.
- Desktop Vulkan workloads: all six passed on llvmpipe, with the Khronos
  layer enabled and zero validation errors. Synchronization validation was
  requested through VK_LAYER_ENABLES. Missing-driver execution returns a
  bounded failed JSON report and a nonzero exit status.
- A temporary negative-control build reuses old source blocks for the second
  draw; only the update/resample test fails, with zero validation errors. The
  mutated source/build stays under ignored reports/ and is not shipped.
- Android Gradle debug build and lint passed with pinned toolchains.
- Original Rust engine cross-built for both Android ABIs; both final ELF files
  export the Rust and JNI engine entry points. Host C-report example executes
  all five guest programs and validates expected traces and handle cleanup.
- APK signature, native ELF architecture and 16 KiB ELF/ZIP load alignment
  verified by tools/verify_gpu_apk.py. This is packaging evidence, not a test
  on a 16 KiB Android system.
- Installed and launched in an isolated Android 15/API 35 x86_64 AVD with
  SwiftShader. All six native workloads passed in 0.3.0-dev. Upgrade from the
  four-test 0.2.0-dev build rejects its old cached report. JSON export matches the
  private saved report byte for byte. A cold restart restores it and enables
  export without rerunning tests. Android validation layers are not bundled.
- The 0.4.0-dev upgrade from cached workload 3 rejects the old report; all four
  original guest programs and all six GPU checks pass through Android/JNI.
  Export matches the retained schema-2 report byte for byte. Cold restart
  restores it without rerunning checks. No app crash or app ANR was captured;
  a System UI startup ANR required its Wait action. This is software x86_64
  evidence, not physical ARM64 evidence. See docs/validation/android-engine.md.
- The 0.5.0-dev AVD run passes all five real guest programs plus six GPU checks.
  The new guest trace matches 97/2/4/3/1/1680 for instructions/threads/switches/
  waits/timeouts/virtual ns. Upgrade invalidation, exact export and cold restore
  pass. A System UI startup ANR blocked the first tap; dismissing that system
  dialog and using fresh XML bounds started the successful run. No app crash/
  ANR captured. See docs/validation/android-scheduling.md.
- Documentation links resolve. Tests and reports are under ignored reports/;
  the copied development APK and hash are under ignored artifacts/.

No physical phone is connected through ADB. The user supplied GPU Lab
0.1.0-dev, 0.2.0-dev and 0.3.0-dev reports from Mali-G720-Immortalis MC12 /
realme RMX5061 / Android 16 / Vulkan 1.3.247, with matching device and driver
identity. The 0.3.0-dev report records all six checks passing, including
converted BC1 texture sampling and update/resampling in one submission without
native BC support. It reports 254688 requested Vulkan bytes / 19 allocations,
excluding driver/pipeline and host heap usage. Validation was disabled. BC1/3/7
remain unsupported natively; ASTC LDR and geometry/tessellation are advertised.
See docs/validation/g720-sampling.md and the earlier validation records.
The physical sampling/update corpus checkpoint is complete. This does not
establish guest graphics, sustained performance or commercial-game support.
The new user-supplied 0.4.0-dev report also passes all four original-engine
cases on physical ARM64 (5/384/529/87 instructions and 0/5/7/0 IPC calls),
with expected debug output, empty errors and zero live handles after each exit.
All six GPU cases still pass; device/driver identity is unchanged. The raw
report stays ignored; see docs/validation/g720-engine.md. This closes the
ARM64 original-engine boundary checkpoint, not the newer scheduling fixture.

## Next implementation work

Architecture, runtime contracts and milestone gates are in docs/architecture.md,
docs/design.md and docs/implementation-plan.md. M1A/M2 satisfy only the owned
static NRO fixture gate; general homebrew compatibility remains open. M1B has
an initial lab, with its physical-device gate and additional experiments open.
M3 has its first single-core multithreaded/blocking-timeout fixture implemented,
alongside handles/IPC/cloning/events. Full synchronization and physical scheduling
evidence remain open. M4 onward is unimplemented. The user
requested autonomous progression after each verified checkpoint; continue
authorized work without repeatedly asking for another go-ahead.

1. Continue kernel cancellation/synchronization and ISA requirements for actual
   guest GPU service submission. Unsupported operations remain explicit errors.
   Keep virtual reference timing distinct from console timing and performance.
2. Obtain a 0.5.0-dev phone report for the new two-thread fixture. Prior engine
   and GPU device checkpoints are passed; the added scheduler needs evidence.
   Do not imply unchanged GPU checks add guest graphics validation.
3. Continue conversion/cache/resource-peak evidence, required BC formats,
   guest tiling, shader-stage experiments and Android presentation separately.
4. Implement guest GPU memory/submission/services, bounded command decoding
   and our shader decoder/IR/SPIR-V path. A native host draw is not this work.
5. Extend the small diagnostic Rust bridge into the Android gameplay host, then add the
   necessary filesystem, audio/input, lifecycle and performance features.
6. Test selected commercial workloads only when their required CPU/services/
   graphics paths exist. Z-A is not ready to boot or test yet.

## Environment

Private repository: https://github.com/sahuishan01/emulator, with source on
`main`. The verified development GPU Lab APK, checksum, packaging report and
artifact README are attached to the `v0.5.0-dev` prerelease. Earlier development
snapshots through `v0.4.0-dev` are retained. APK binaries and
private reports remain outside source control. This is a prototype snapshot,
not a production release or a Switch compatibility claim.

A phone browser later reported ERR_INVALID_RESPONSE at a temporary GitHub
asset redirect. The supplied URL was expired at investigation time. A fresh
authenticated asset request returned HTTP 302 followed by HTTP 200 with the
expected APK hash. This does not establish the browser/network root cause.
The same private prerelease now includes `emulator-gpu-lab.zip` and its SHA-256
file as an alternative download. README links use permanent GitHub asset URLs;
never preserve or publish temporary signed redirects.

Rust/Cargo 1.85, LLVM/Clang 19, Vulkan headers/loader, Mesa llvmpipe,
Khronos validation layers, glslangValidator and spirv-val are available.
Android SDK is under ~/Android/Sdk. JDK 21 is installed; the build targets
Java 17. Android pins Gradle 8.9, AGP 8.7.3, Kotlin 2.0.21, SDK 35,
build-tools 34.0.0, NDK 28.2.13676358 and CMake 3.22.1.
Android Rust uses named rustup 1.85.1 plus both Android targets; the helper sets
RUSTC explicitly because Debian's system compiler may precede rustup in PATH.
The global rustup default is not changed.

The QA AVD is isolated under reports/android-qa/avd, separate from existing
user AVDs. Shut down only emulator-5580 after checks. Do not change another
emulator/device without authorization. Keep captures and APKs outside Git.
The existing tools/collect_android_gpu.py remains an optional diagnostic.
