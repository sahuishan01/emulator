# Android guest draw command checkpoint

0.18.0-dev/versionCode18; final APK SHA-256:
`f1a1c187c6f8315d9a8024a7274fd5011240a184e04758a9451fd50b4f73516d`.

183 Rust tests pass, including five direct draw tests, the interpreted draw NRO,
its owned value ABI/oracles, fifty actual kernel/NV scenarios and nineteen shader
reference cases. Formatting and Clippy with warnings denied pass. All26 LLVM
fixtures (11raw/15NRO), three clear command binaries, three new draw command/page
pairs, four raw shader/disassembly fixtures and host GLSL/SPIR-V artifacts match.
The four modules emitted by our Rust translator pass Khronos spirv-val Vulkan1.1;
NVIDIA nvdisasm12.8.55 SM53 independently matches the raw fixture listings.

The new guest's actual trace:5571 instructions/25 IPC/2 copied handles/18 A/11 B/
603 descriptor bytes; full128-pixel formula, completion2, transfer marker42 and
zero resources. Header/vertex/draw state is consumed through actual NV commands.
Reference group sizes: engine5683/NV6557/core642/shader2891 bytes, all within ABI2.
Prior engine14/NV34/core3/shader15 cases match0.17 in every per-case field. New
shader totals282 instructions/2842 words/30 invocation oracles/1655 rejections,
including1280 header bit mutations. See [the exact subset](../guest-draw-batch.md).

Desktop llvmpipe passes all15 Vulkan workloads with validation enabled and zero
errors. The first12 case records match0.17. New full512-byte comparisons cover
actual interpreted NRO command replay, owned BGRA draw replay and decoded scissor
replay. Shader modules, vertices, target/scissor and draw counts derive from the
original guest command preparation. This is offscreen replay after synchronous
CPU completion, not a live/asynchronous renderer or display handoff.

Pinned Rust1.85.1 ARM64/x86_64 build, Gradle assemble/lint, APK signature, native
machine identities, all seven Rust exports in both libraries and16KiB ELF/ZIP
alignment pass. The new replay ABI is50452 bytes/alignment4; record size16536 and
all critical offsets are asserted by Rust/C++. Packaging does not establish a
16KiB Android runtime test. No emulator core or third-party shader translator is
used. An off-grid vertex viewport-rounding case was tightened before the final
APK build; only the final APK hash above is published.

Android15/API35 x86_64 SwiftShader passes all102 cases in one run:
engine15/NV50/GPU-core3/shader19/Vulkan15. All four Rust groups match fresh host
reports except architecture. Summary derives and displays all five group counts.
Upgrade invalidates workload17; schema4/workload18 requires every group. Export
writes byte-exact JSON; force-stop/cold launch restores the same bytes, summary
and enabled Export. Both screenshots were visually inspected. No app crash/ANR;
crash buffer empty.

Owned AVD started after98.42% CPU idle, nice10/two cores/2048MiB/headless
SwiftShader/no snapshot. A System UI startup ANR Wait was handled through its
fresh XML button and recorded in QA notes. Two empty picker XML attempts were
retried successfully. Native checks ran once; export/restore completed without
restarting the suite. QA returned0, stopped only emulator-5580 and its process
was awaited. Reports, artifacts, logs and captures remain ignored.

The supplied [0.17 G720 report](g720-shader-translation-batch.md) passes all78
previous phone checks. This expanded102-case batch awaits one physical report.
Presentation, live backend completion, broader GPU/runtime/services, commercial
startup and Z-A remain unfinished. Debug-signed development diagnostic.
