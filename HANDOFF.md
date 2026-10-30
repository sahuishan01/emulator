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

- Six original Rust workspace crates: memory, a64, loader, kernel, session and
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
- Original C++ Vulkan GPU Lab shared by desktop CLI and Android JNI. Three
  offscreen workloads: exact image-transfer roundtrip, overlapping depth-tested
  triangles and compute-generated vertices compared to the CPU-uploaded draw.
- GPU reports contain device/driver identity, physical feature advertisements,
  limits, heaps, queues, 11 format queries and per-workload results. A format
  query is not proof of texture sampling/decoding correctness.
- Kotlin Android GPU Lab with background native checks, retained report and
  user-selected JSON export. ARM64 and x86_64 debug APK. This is a separate
  diagnostic app; it does not embed or execute the Rust engine.

## Verified

- `cargo test --offline --workspace`: 34 tests passed.
- All 11 fixture binaries match LLVM 19 assembly/link output, independently of
  our decoder. Sources include the original homebrew NRO.
- Rust formatting and Clippy with `-D warnings`: passed.
- GLSL generation and spirv-val checks: passed.
- Desktop Vulkan workloads: all three passed on llvmpipe, with the Khronos
  layer enabled and zero validation errors. Synchronization validation was
  requested through VK_LAYER_ENABLES. Missing-driver execution returns a
  bounded failed JSON report and a nonzero exit status.
- Android Gradle debug build and lint passed with pinned toolchains.
- APK signature, native ELF architecture and 16 KiB ELF/ZIP load alignment
  verified by tools/verify_gpu_apk.py. This is packaging evidence, not a test
  on a 16 KiB Android system.
- Installed and launched in an isolated Android 15/API 35 x86_64 AVD with
  SwiftShader. All three native workloads passed. JSON export matches the
  private saved report byte for byte. A cold restart restores it and enables
  export without rerunning tests. Android validation layers are not bundled.
- Documentation links resolve. Tests and reports are under ignored reports/;
  the copied development APK and hash are under ignored artifacts/.

No physical phone is connected. The G720 gate is untested. An AVD/software
renderer pass is not physical Mali validation or Switch gameplay evidence.

## Next implementation work

Architecture, runtime contracts and milestone gates are in docs/architecture.md,
docs/design.md and docs/implementation-plan.md. M1A/M2 satisfy only the owned
static NRO fixture gate; general homebrew compatibility remains open. M1B has
an initial lab, with its physical-device gate and additional experiments open.
M3 onward is unimplemented.

1. Run the built GPU Lab APK on the user's phone, export JSON and record the
   exact APK hash, phone/Android/GPU/driver and workload results.
2. Extend the lab with a tested reference texture decoder/GPU fallback,
   sampled textures, missing-stage experiments and Android presentation.
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
artifact README are attached to the `v0.1.0-dev` prerelease. APK binaries and
private reports remain outside source control. This is a prototype snapshot,
not a production release or a Switch compatibility claim.

Rust/Cargo 1.85, LLVM/Clang 19, Vulkan headers/loader, Mesa llvmpipe,
Khronos validation layers, glslangValidator and spirv-val are available.
Android SDK is under ~/Android/Sdk. JDK 21 is installed; the build targets
Java 17. Android pins Gradle 8.9, AGP 8.7.3, Kotlin 2.0.21, SDK 35,
build-tools 34.0.0, NDK 28.2.13676358 and CMake 3.22.1.

The QA AVD is isolated under reports/android-qa/avd, separate from existing
user AVDs. Shut down only emulator-5580 after checks. Do not change another
emulator/device without authorization. Keep captures and APKs outside Git.
The existing tools/collect_android_gpu.py remains an optional diagnostic.
