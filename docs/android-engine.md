# Android original-engine checks

Version 0.15.0-dev embeds the original Rust engine into the existing Android
diagnostic app. The CPU backend interprets guest A64 instructions on both host
architectures. It does not execute guest bytes as host machine code.

The app's worker calls Rust once to execute thirteen original NRO fixtures
described in [kernel-ipc.md](kernel-ipc.md), runs eighteen integrated NV protocol scenarios, separately runs three actual GPU-core
memory/command cases, including original DMA execution, then calls the unchanged six-test
C++ Vulkan suite. The Rust checks validate actual debug output, instruction/IPC
counts, guest memory results and zero remaining handles after process exit.
The result is not hardcoded successful output. Each native group reports its
own failure; one failed group does not discard the other's diagnostic report.

## Native boundary

`crates/android-ffi` builds as a static library, linked into the existing
`libemulator_gpu_lab.so`. C exports are `emulator_run_reference_guests_v2()` and
`emulator_run_gpu_core_checks_v2()` plus `emulator_run_nv_checks_v2()`, each returning EngineReport. The C/Rust layout is a
8200-byte, 4-byte-aligned value: u32 ABI version, u32 byte length, and 8192
initialized payload bytes. Rust returns owned bytes by value, so callers never
receive a Rust pointer or allocation and need no free callback. JNI validates
ABI version/capacity before creating the Java byte array. ABI 2 increases the
payload from 4096 to 8192 bytes for the eleventh case. All exports use `_v2`
symbols, so stale ABI 1 callers fail to link instead of accepting an incompatible
return layout. Rust/JNI rebuild together; size, alignment and offset are checked.

Core crates still forbid unsafe Rust. This boundary permits the no-mangle
export attribute only and contains no unsafe blocks or raw-pointer arguments.
Rust panics are caught at the exported function and produce a bounded failed
report. Allocation failure or a process/driver crash is not isolated by this
mechanism. A future gameplay API needs lifecycle/session/surface ownership;
the reference-report ABI is deliberately smaller.

## Build

The Gradle task builds both Android Rust archives before configuring CMake.
Pins: Rust 1.85.1; NDK 28.2.13676358; Android minimum API 31; Gradle 8.9,
AGP 8.7.3, Kotlin 2.0.21, CMake 3.22.1, compile/target SDK 35 and build-tools
34.0.0. JDK 17 or 21 works. The cross-build helper currently supports Linux
hosts and uses two Cargo jobs. It sets RUSTC explicitly to the named rustup
toolchain, even when the system compiler appears first in PATH. It does not
change the user's global Rust default.

```sh
rustup toolchain install 1.85.1 --profile minimal --target aarch64-linux-android --target x86_64-linux-android
cd android
./gradlew :gpu-lab:assembleDebug :gpu-lab:lintDebug --no-daemon
```

The SDK defaults to `~/Android/Sdk`, with ANDROID_SDK_ROOT/ANDROID_HOME overrides.
Archives are under `target/android-engine/<abi>/`; the APK is
`android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk`. Packaging checks
verify signature, ARM64/x86_64 ELF identity and 16 KiB ELF/ZIP alignment:

```sh
python3 tools/verify_gpu_apk.py android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk
cargo run --offline -p emulator-android-ffi --example reference_report
cargo run --offline -p emulator-android-ffi --example reference_report -- gpu-core
```

The host example executes the same checks without JNI. Passing it does not
validate the Android C ABI or either Android runtime architecture.

## Report and device checkpoint

The package/application ID stays `com.algosculptor.emulator.gpulab` so existing
installations update in place. Select **Run all checks**, then
**Export report**. Schema 4/workload 15 separates `engine`, `nv`, `gpuCore` and `gpu`,
with combined `allPassed` requiring all four groups. The embedded Vulkan suite retains
its own workload version 3. Engine reports record host/guest architecture,
thirteen per-case outcomes, instruction/IPC counts, output, handle cleanup and
thread/switch/wait/timeout/cancellation/virtual-time counters. Engine suite 10 adds a combined channel submission/completion guest
(7179 instructions/32 IPC/2 copies/25 A/18 B/875 bytes). The separate NV suite 1
runs eighteen kernel API scenarios; see [the complete batch](nv-channel-batch.md).
Engine suite 9 adds the NV GPU mapping guest (3172 instructions/15 IPC/
2 copies/9 A/6 B/372 bytes); see [the mapping subset](nv-gpu-address-space.md).
Engine suite 8 adds the 817-instruction/10-IPC backing-import guest
(5 A/4 B/162 bytes/two copies) with Alloc/ALIGNMENT/Free checks.
Engine suite 7 adds a 719-instruction/nine-IPC NVMap guest
(four A/three B/98 bytes/two copied handles), validating device lifecycle and
unallocated-object Create/SIZE/Free. See [the device subset](nvmap-devices.md).
Suite 6 adds copyHandles and the transfer-memory/NV initialization fixture (317
instructions/four IPC calls/two copied handles/restored CPU rights). Suite 5
adds successful IPC sendBuffers/receiveBuffers/bufferBytes and a buffered DMA
fixture (327 instructions/four IPC calls/three A/one B/80 bytes). Earlier suite 4
added inline guest DMA and suite 3 cancellation. Both DMA guests discover
`emu:gpu`, submit real commands, verify bytes/counters and close their handles. `cancelledWaits` counts delivered interruptions;
`timeouts` counts expired blocked waits, excluding immediate zero-timeout polls.
The top level adds device, Android and application identity. Previous
workload-version reports are not shown as current results after an upgrade.

The isolated Android 15/API 35 x86_64 AVD is the software QA target; its
SwiftShader results do not establish ARM64/Mali behavior. The physical G720
0.4.0-dev report passed the four engine checks through the ARM64 C ABI as well
as all six unchanged GPU checks. See [physical evidence](validation/g720-engine.md).
The [0.5.0-dev physical report](validation/g720-scheduling.md) also passes the
fifth, scheduling/blocking-wait fixture. The [0.6.0-dev report](validation/g720-cancellation.md)
also passes cancellation. The `gpuCore` suite 2 verifies shared CPU/GPU
aliases and queued-command capture/resource lifetimes through Rust/JNI. These
are direct engine API checks, not NRO or NV-service submissions. Its two cases
passed in the [0.7.0-dev physical report](validation/g720-gpu-ownership.md).
A third case exercises original DMA method execution: 11 writes, two dependent
launches, 16 copied bytes and zero remaining backing/pending work. That is the
checkpoint verified in the [0.8.0-dev phone report](validation/g720-dma.md).
Guest DMA submission also passes in the
[0.9.0-dev phone report](validation/g720-guest-dma.md). The new phone checkpoint
was engine suite 5's buffered NRO, which passes in the
[0.10.0-dev report](validation/g720-hipc-buffers.md). The
[0.12.0-dev phone report](validation/g720-nvmap.md) also passes the transfer-memory
and unallocated NVMap guests. The [0.13.0-dev phone report](validation/g720-nvmap-backing.md)
also passes backing imports and ABI 2: all twenty prior cases. The [0.14.0-dev phone report](validation/g720-nv-gpu-map.md) also passes mapping:
all twenty-one prior cases. The new phone checkpoint batches channel setup,
submission, completion and failure/lifetime checks.

Broader GPU method execution, shader translation, guest-frame presentation and
commercial startup remain open. Pokémon Legends: Z-A is a later target.


The combined NV batch can be verified on the owned AVD in one command after boot:

```sh
python3 tools/qa_android_batch.py --apk android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk --output reports/android-qa/v0.15.0-dev --version 0.15.0-dev --workload 15 --previous-workload 14 --engine-reference reports/v0.15-host-engine.json --nv-reference reports/v0.15-host-nv.json --core-reference reports/v0.15-host-gpu-core.json --shutdown
```

Generate each reference via the reference_report example (default, gpu-core, nv).
The upgrade flow expects a cached successful prior-workload report. `--resume`
revalidates an already completed current batch, then exports and checks cold
restore; it retains the original upgrade evidence. Fresh XML/expected-screen
polling handles asynchronous document saving; it never taps inferred coordinates.
