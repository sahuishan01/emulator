# Android shader translation checkpoint

0.17.0-dev/versionCode17; APK SHA-256:
`c255fced3a8ed403b4ab518c7cb6533096b23a4dc7728e7d7fc004ee49e4d0cc`.

176 Rust tests pass, including four shader tests (the portable fifteen-scenario
runner, every truncation, 2048 single-bit mutations and SSA register overwrite)
and two new ABI/report tests. Formatting and Clippy with warnings denied pass.
All25 LLVM fixtures, three clear command fixtures and existing GLSL/SPIR-V
artifacts match. Four original shader binaries/readable listings match NVIDIA
nvdisasm12.8.55 SM53 exactly. All four modules emitted by our Rust translator
pass Khronos spirv-val for Vulkan1.1. See [the exact subset](../shader-translation-batch.md).

Desktop llvmpipe passes all12 Vulkan cases with Khronos validation enabled and
zero errors. The three new checks consume actual translated SPIR-V and compare
all512 image bytes independently: fullscreen green, scaled red/BGRA with black
border, and green/red pipeline changes with scoped coverage in one submission.
The prior nine workload outcomes remain passing. Original engine/NV/GPU-core
host reports are unchanged from0.16; new shader report is2270 bytes and records
273 decoded instructions/2588 emitted words/28 invocation oracles/117 rejections.

Pinned Rust1.85.1 ARM64/x86_64 build, Gradle build/lint, signature, native machine
identity, all six Rust exports in both libraries, and16KiB ELF/ZIP alignment pass.
The new shader value ABI is17264 bytes/alignment4, with Rust/C++ checked offsets;
EngineReport remains ABI2/8200 bytes/8192 payload. Packaging is not proof of an
Android16KiB runtime test. No third-party emulator core, shader translator or
NVIDIA tool binary is part of the application or build dependency graph.

Android15/API35 x86_64 SwiftShader AVD passes all78 checks in one run: engine14,
NV34, GPU-core3, shader15, Vulkan12. Every field in all four Rust groups matches
fresh host reference reports except architecture. The visible summary displays
all five group counts and enables Export. Upgrade invalidates workload16; the
new schema4/workload17 requires every group. Export creates byte-exact JSON;
force-stop/cold launch restores the same bytes, summary and enabled Export.
Both screenshots were visually inspected. No app crash/ANR; crash buffer empty.

The owned AVD started after97.57% CPU idle, nice10/two cores/2048MiB/headless
SwiftShader/no snapshot. A System UI startup ANR was handled using its current
XML Wait button, explicitly retained in QA notes. No app ANR or harness retry
was hidden. The full QA command returned0, shut down only emulator-5580 and its
process was awaited. Raw reports/captures/tool binaries/logs remain ignored.

The [supplied0.16 G720 report](g720-color-clear-batch.md) passes all60 previous
phone cases. The later [supplied0.17 G720 report](g720-shader-translation-batch.md) passes all78
checks on the physical phone. Host draw setup
still supplies stage metadata, vertex layout and draw/scissor calls; guest
program headers, draw commands, live renderer/display integration, game startup
and Z-A remain unfinished. Debug-signed development diagnostic.
