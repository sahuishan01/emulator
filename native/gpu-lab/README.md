# Original Vulkan GPU laboratory

This is original host-side diagnostic code shared between a desktop CLI and
the Android GPU Lab application. It does not emulate Tegra commands, translate
guest shaders or run Switch games. The C++ implementation uses the NDK/system
Vulkan interfaces. The Android package also links the original Rust workspace
for CPU/kernel/IPC reference checks; the desktop Vulkan runner remains independent.

Six bounded offscreen workloads (workload version 3):

1. RGBA8 buffer -> image -> buffer transfer with exact byte comparison.
2. Two overlapping triangles with depth testing. The nearer green triangle
   must occlude a later red triangle, and a corner must retain its clear color.
3. A compute shader generates the same vertex/color data, followed by an
   explicit compute-to-vertex dependency. Every output byte must match the
   CPU-uploaded draw's result.
4. An original compute shader decodes BC1 RGBA UNORM bytes into an RGBA8 storage
   image without native BC support. Every readback byte must match a fixture
   produced by our tested Rust reference decoder. Both opaque and transparent
   block modes are exercised. See [the decoding contract](../../docs/bc1-fallback.md).
5. Sample the converted RGBA8 image in a fullscreen draw, checking nearest
   coordinates, mirrored orientation, clamped edges and transparent pixels
   against a CPU reference for every output byte.
6. Reconvert the same image from changed BC1 input and sample again. Both
   dispatch/draw pairs run in one submission, with both encoded inputs retained
   unchanged. See [the sampling and reuse contract](../../docs/bc1-sampling.md).

The report includes device/driver/API identity, software-renderer status,
physical feature advertisements, limits, memory heaps, queue families and
format-query results, plus per-test outcomes and errors. Format entries report
optimal-tiling feature bits and a combined 2D sampled/transfer-source/transfer-
destination image query. Heap sizes are advertised capacity, not free memory
or the application's allocation budget. A passing report
establishes these workloads only. Compressed texture sampling, formats beyond
linear BC1 RGBA UNORM, stencil,
additional shader stages, Android swapchain presentation and guest GPU command
handling are not established by these tests.

The allocation section records device-memory requests across all workloads,
which remain allocated through teardown. It excludes driver/pipeline and
process/Java/native heap overhead; it is not an application memory budget.

Our GLSL sources are compiled and validated into the committed aligned SPIR-V
word arrays in `generated/shaders.h`. Regenerate/verify with:

```sh
python3 tools/compile_gpu_shaders.py
python3 tools/compile_gpu_shaders.py --check
python3 tools/generate_bc1_fixture.py --check
```

Requirements: glslangValidator and spirv-val. Native desktop builds additionally
need Vulkan headers/loader, CMake 3.22+, Ninja and a C++17 compiler:

```sh
cmake -S native/gpu-lab -B target/gpu-lab -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build target/gpu-lab --parallel 2
target/gpu-lab/gpu-lab-cli
```

To require the installed Khronos validation layer on the desktop:

```sh
EMULATOR_GPU_VALIDATION=1 target/gpu-lab/gpu-lab-cli
```

The JSON explicitly records whether validation was enabled and its error count.
Use a software ICD explicitly for CI/headless checks if required; software
results do not establish physical Mali behavior.

Android build pins Gradle 8.9, AGP 8.7.3, Kotlin 2.0.21, NDK 28.2.13676358,
CMake 3.22.1 and SDK 35/build-tools 34.0.0. Use JDK 17 or 21. With the current
Android CLI, missing native packages can be installed using
`android sdk install ndk/28.2.13676358` and `android sdk install cmake/3.22.1`.
Older SDK managers use their corresponding semicolon package IDs.

The Android package also embeds our original Rust engine. Install rustup's
`1.85.1` toolchain with `aarch64-linux-android` and `x86_64-linux-android` targets.
Gradle runs `tools/build_android_engine.py` before CMake; that helper currently
supports Linux hosts. See [the Android boundary](../../docs/android-engine.md).

```sh
cd android
./gradlew :gpu-lab:assembleDebug :gpu-lab:lintDebug --no-daemon
```

The APK is `android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk`. It has a
separate package ID, `com.algosculptor.emulator.gpulab`, with arm64-v8a and
x86_64 native libraries. It uses the system driver and requires no network or
storage permissions. Run checks, then use Export report to select a JSON
destination. The report adds the phone, SoC, Android and application versions.
Version 0.7.0-dev uses schema 3/workload 7: `engine` has six guest results,
`gpuCore` has two Rust shared-memory/command-capture cases, and `gpu` has these
six unchanged native Vulkan results (suite version 3). Combined `allPassed`
requires all three groups. GPU-core checks are direct engine calls, not guest
GPU execution or Vulkan rendering.
The last completed report is retained across activity recreation.
Reports from an older workload version are not displayed as current results;
after upgrading from an earlier workload version, run the updated checks again.

Verify signatures, native architectures, ELF load alignment and native ZIP
payload alignment with:

```sh
python3 tools/verify_gpu_apk.py android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk
```

The native workload currently executes within the app process. Its fence wait
has a timeout, but teardown may still depend on a responsive driver. Isolation
against a driver that hangs during teardown is future work; this lab is a
development build, not a production graphics backend.
