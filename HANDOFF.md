# Handoff

## Scope and target

The user wants a new emulator written from scratch. The earlier Eden/Lemon
patching approach is superseded. Do not fork, link, import or copy their cores.
Untouched `upstream/` references remain ignored and outside all builds.

Target Switch 1 first, then other consoles. Primary phone: Realme GT 7 Dream
Edition / Dimensity 9400e / Immortalis-G720. Pokémon Legends: Z-A is a later
compatibility target. The earlier black screen in other emulators is not a
root-cause diagnosis for our code. No commercial game runs in this implementation.

## Implemented on 2026-10-30

- Seven original Rust workspace crates: memory, a64, loader, kernel, session, texture and
  CLI. No external Cargo dependencies; Rust unsafe code remains forbidden.
- Bounded memory with transactional batch mappings, reservation/commit,
  protect/unmap and cross-region accesses. Failed writes do not partially
  mutate memory. Protection/unmapping currently copy backing data; shared
  backing/alias and page models remain future work.
- A64 integer interpreter subset, including ADR/ADRP and wide-immediate forms.
  Unsupported instructions and services return explicit faults.
- Bounded static uncompressed NRO0 v0 subset, RX/R/RW segments, BSS, guarded
  stack and minimal homebrew entry configuration. MOD0/linking, NSO and full
  homebrew runtimes are unsupported. See docs/nro-loader.md.
- OutputDebugString and ExitProcess SVC dispatch with bounded raw debug
  records. There is one execution context, no scheduler/handle operations or
  IPC. Invalid debug buffers fault this strict prototype rather than delivering
  complete Horizon error-result behavior.
- Session execution budgets pause and resume without repeating debug output.
  Our own hello.nro prints its known message and exits after five instructions.
  The raw CPU demo still produces 42 after 24 instructions.
- Original C++ Vulkan GPU Lab shared by desktop CLI and Android JNI. Four
  offscreen workloads: exact image-transfer roundtrip, overlapping depth-tested
  triangles, compute-generated vertices compared to the CPU-uploaded draw,
  and BC1-to-RGBA8 compute conversion compared to our Rust reference decoder.
- Original bounded BC1 RGBA UNORM reference decoder, six tests and generated
  GPU corpus. Does not implement other BC formats, sRGB, guest tiling or sampling.
- GPU reports contain device/driver identity, physical feature advertisements,
  limits, heaps, queues, 11 format queries and per-workload results. A format
  query is not proof of texture sampling/decoding correctness.
- Kotlin Android GPU Lab with background native checks, retained report and
  user-selected JSON export. ARM64 and x86_64 debug APK. This is a separate
  diagnostic app; it does not embed or execute the Rust engine.

## Verified

- `cargo test --offline --workspace`: 40 tests passed.
- All 11 fixture binaries match LLVM 19 assembly/link output, independently of
  our decoder. Sources include the original homebrew NRO.
- Rust formatting and Clippy with `-D warnings`: passed.
- GLSL generation and spirv-val checks: passed.
- Desktop Vulkan workloads: all four passed on llvmpipe, with the Khronos
  layer enabled and zero validation errors. Synchronization validation was
  requested through VK_LAYER_ENABLES. Missing-driver execution returns a
  bounded failed JSON report and a nonzero exit status.
- Android Gradle debug build and lint passed with pinned toolchains.
- APK signature, native ELF architecture and 16 KiB ELF/ZIP load alignment
  verified by tools/verify_gpu_apk.py. This is packaging evidence, not a test
  on a 16 KiB Android system.
- Installed and launched in an isolated Android 15/API 35 x86_64 AVD with
  SwiftShader. All four native workloads passed in 0.2.0-dev. Upgrade from the
  three-test 0.1.0-dev build rejects its old cached report. JSON export matches the
  private saved report byte for byte. A cold restart restores it and enables
  export without rerunning tests. Android validation layers are not bundled.
- Documentation links resolve. Tests and reports are under ignored reports/;
  the copied development APK and hash are under ignored artifacts/.

No physical phone is connected through ADB. The user supplied GPU Lab
0.1.0-dev and 0.2.0-dev reports from Mali-G720-Immortalis MC12 / realme RMX5061 /
Android 16 / Vulkan 1.3.247, with matching device and driver identity. The new
0.2.0-dev report records all four checks passing, including BC1-to-RGBA8
conversion without native BC support. BC1/3/7 queries remain unsupported,
ASTC LDR and geometry/tessellation remain advertised. Validation was disabled.
See docs/validation/g720-initial.md and docs/validation/g720-bc1.md. This closes
the physical conversion-corpus checkpoint; sampling and commercial gameplay
are not established by the report.

## Next implementation work

Architecture, runtime contracts and milestone gates are in docs/architecture.md,
docs/design.md and docs/implementation-plan.md. M1A/M2 satisfy only the owned
static NRO fixture gate; general homebrew compatibility remains open. M1B has
an initial lab, with its physical-device gate and additional experiments open.
M3 onward is unimplemented.

1. Extend the BC1 prototype to sample its converted RGBA8 image in a draw and
   compare rendered readback, including transparency/edges, compute-to-fragment
   ordering and lifetime. The existing four-test physical checkpoint passed;
   do not ask the user to repeat it without a changed workload.
2. Add conversion-update/cache and memory-peak evidence, then other required
   BC formats, guest tiling, shader-stage experiments and Android presentation.
3. Add original threads, handles, events, waits, timers and bounded IPC with
   owned guest workloads. Expand ISA and loaders only as required by them.
4. Implement guest GPU memory/submission/services, bounded command decoding
   and our shader decoder/IR/SPIR-V path. A native host draw is not this work.
5. Integrate the Rust engine into the Android gameplay host, then add the
   necessary filesystem, audio/input, lifecycle and performance features.
6. Test selected commercial workloads only when their required CPU/services/
   graphics paths exist. Z-A is not ready to boot or test yet.

## Environment

Private repository: https://github.com/sahuishan01/emulator, with source on
`main`. The verified development GPU Lab APK, checksum, packaging report and
artifact README are attached to the `v0.2.0-dev` prerelease. The original
`v0.1.0-dev` snapshot is retained. APK binaries and
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

The QA AVD is isolated under reports/android-qa/avd, separate from existing
user AVDs. Shut down only emulator-5580 after checks. Do not change another
emulator/device without authorization. Keep captures and APKs outside Git.
The existing tools/collect_android_gpu.py remains an optional diagnostic.
