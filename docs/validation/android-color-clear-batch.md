# Android color-clear and Vulkan replay checkpoint

0.16.0-dev/version code 16; APK SHA-256:
`8c33048fb6b38c045b99029b54e5bfea5a09e669269468ea731d6d092718f778`.

170 Rust tests pass, including four direct clear transaction/ordering/range tests,
the fixed-value replay ABI test and the interpreted color-clear NRO. The NV
runner now executes 34 independent protocol scenarios, sixteen added in this
batch. Formatting and Clippy with warnings denied pass. All 25 LLVM fixtures
(11 raw/14 NROs), three generated command fixtures, and GLSL/SPIR-V validation
match checked-in artifacts. See [the exact supported subset](../color-clear-batch.md).

Desktop llvmpipe passes all nine Vulkan cases with Khronos validation enabled
and zero errors. Three new cases compare every output byte: actual interpreted
NRO command replay/RGBA, owned BGRA replay and masked overlapping replay. Fixed
host shaders lower color clears; this is not guest shader translation.

Pinned Rust 1.85.1 ARM64/x86_64 cross-build, Gradle build/lint, APK signature,
native machine identity, four exported symbols and 16 KiB ELF/ZIP alignment
pass. EngineReport ABI 2 remains 8200 bytes with an 8192-byte payload. The new
clear value ABI is 1264 bytes/alignment 4, with checked offsets on both sides.
Host report sizes: engine 5302/NV 4427/GPU-core 642 bytes. Packaging alignment
does not establish a 16 KiB Android runtime test.

Android 15/API 35 x86_64 AVD passes all 60 checks in one run:
engine 14 (suite 11), NV 34 (suite 2), GPU-core 3 (suite 2), Vulkan 9 (workload 4).
All three Rust groups match fresh host references in every field except
architecture. The visible summary shows 60/60 and all four group counts.
Schema 4/workload 16 requires all groups. The upgrade invalidates workload 15;
one new run enables Export, which writes byte-exact JSON. Force-stop/cold launch
restores the same bytes, summary and enabled Export. Both screenshots were
visually inspected. No app crash/ANR appears in captured logs; crash buffer empty.

The owned AVD started only after an idle sample of 98.64%, using nice 10/two
cores/2048 MiB/headless SwiftShader/no snapshot. Cold boot took 96 seconds.
At startup a System UI ANR dialog was handled using its current XML Wait
button; this is explicitly retained in verification notes. Two initial picker
XML attempts returned no completion; bounded fresh-dump retries succeeded.
There was no app ANR or rerun of the native suite. The full QA command completed
successfully, explicitly stopped only emulator-5580, and its process was awaited.
No Android target remains connected. Logs/captures/raw reports remain ignored.

The supplied [0.15 G720 report](g720-nv-channel-batch.md) confirms all 40 previous
phone cases. The later [supplied 0.16 phone report](g720-color-clear-batch.md) passes all60
checks on the physical G720. It is
offscreen command replay and CPU reference execution, not a live asynchronous
renderer, guest shader/draw support, display presentation, game compatibility
or console performance. Debug-signed development diagnostic.
