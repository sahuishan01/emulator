# Original Vulkan GPU laboratory

This is original host-side diagnostic code shared between a desktop CLI and
the Android app's support diagnostics. The linked original Rust engine decodes narrow
owned Tegra clear/draw commands and shaders, with a synchronous live Catch path.
General guest shaders and commercial Switch games remain unsupported. The C++ implementation uses the NDK/system
Vulkan interfaces. The Android package also links the original Rust workspace
for CPU/kernel/IPC reference checks; the desktop Vulkan runner now links the same Rust replay archive.

37 bounded workloads (native workload version 12). The first nine are:

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

7. Replay actual command bytes from the interpreted NV color-clear NRO through
   our decoder and Vulkan clear lowering; all 512 bytes match the guest image
   and independent CPU formula.
8. Replay a BGRA target variant, checking storage order and rectangles.
9. Replay masked clears; preserve prior red/blue/alpha in overlapping regions.

See [the full clear/replay contract](../../docs/color-clear-batch.md). Fixed host
shaders lower color clears. Three additional workloads consume our original
SM53 translator output: full RGBA draw, transformed red BGRA draw and ordered
green/red pipeline changes. Cases 13–15 replay decoded guest draws. Case 16 runs
111 live Catch guest frames without a window, including score/miss/reset. Cases17–19 add actual guest sampled RGBA/BGRA/updated-texture draws; see [the texture batch](../../docs/texture-draw-batch.md).
General guest display integration remains open.
Cases 20–28 extend indexed geometry and tiled/compressed textures; cases 29–37
add all depth comparisons and disabled writes. See the [bounded depth contract](../../docs/depth-batch.md).
See [the supported shader subset](../../docs/shader-translation-batch.md).

The report includes device/driver/API identity, software-renderer status,
physical feature advertisements, limits, memory heaps, queue families and
format-query results, plus per-test outcomes and errors. Format entries report
optimal-tiling feature bits and a combined 2D sampled/transfer-source/transfer-
destination image query. Heap sizes are advertised capacity, not free memory
or the application's allocation budget. A passing report
establishes these workloads only. Compressed texture sampling, formats beyond
linear BC1 RGBA UNORM, stencil,
additional shader stages, Android swapchain presentation and general guest GPU command
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
cargo build --offline -p emulator-android-ffi
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
retained package ID, `com.algosculptor.emulator.gpulab`, with arm64-v8a and
x86_64 native libraries. It uses the system driver and requires no network or
storage permissions. Open Settings → Diagnostics, run checks, then use Export report to select a JSON
destination. The report adds the phone, SoC, Android and application versions.
Version0.21.1-dev uses schema4/workload23: engine16, nv50, gpuCore11, shader25
and gpu19 Vulkan cases. All five groups must pass (121 total). Native cases
consume SPIR-V emitted by our original SM53 translator: full RGBA output,
transformed BGRA geometry and ordered draws with two different fragment shaders.
The three new guest-command replays derive shaders, vertices, target and scissor
from actual B197 commands and SPH headers, including an interpreted NRO.
See [the draw contract](../../docs/guest-draw-batch.md) and [the live profile](../../docs/live-homebrew.md).
See [the exact subset](../../docs/shader-translation-batch.md). EngineReport ABI2
is retained; fixed clear and shader value ABIs have checked Rust/C++ offsets.
The last completed report is retained across activity recreation.
Reports from an older workload version are not displayed as current results;
after upgrading from an earlier workload version, run the updated checks again.

Verify signatures, native architectures, ELF load alignment and native ZIP
payload alignment with:

```sh
python3 tools/verify_gpu_apk.py android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk --main-app
```

The native workload currently executes within the app process. Its fence wait
has a timeout, but teardown may still depend on a responsive driver. Isolation
against a driver that hangs during teardown is future work; this lab is a
development build, not a production graphics backend.

## Live homebrew (0.19)

The app now presents our own Catch NRO through live NV submissions, original shader
translation and a synchronous Vulkan completion callback. TextureView/window,
per-image present synchronization and touch controls are implemented. The normal
batch now has19 Vulkan cases/121 total; the new111-frame test runs without a window.
Visible display/control/lifecycle QA is separate. See [the exact contract](../../docs/live-homebrew.md).
The previous replay tests remain unchanged; general display queues and games are open.

The 0.20 launcher is a library rather than the diagnostics screen. Its import,
text execution, settings and lifecycle contracts are in [android-app.md](../../docs/android-app.md).
