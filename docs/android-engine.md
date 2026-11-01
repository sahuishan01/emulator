# Android original-engine checks

Version 0.16.0-dev embeds the original Rust engine into the existing Android
diagnostic app. The CPU backend interprets guest A64 instructions on both host
architectures. It does not execute guest bytes as host machine code.

The app's worker calls Rust once to execute fourteen original NRO fixtures
described in [kernel-ipc.md](kernel-ipc.md), runs thirty-four integrated NV protocol scenarios, separately runs three actual GPU-core
memory/command cases, including original DMA execution, then calls the nine-test
C++ Vulkan suite (six prior native cases plus three owned clear replays). The Rust checks validate actual debug output, instruction/IPC
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

Schema 4/workload 16 keeps engine/nv/gpuCore/gpu and requires all four for
combined allPassed. One Run all checks tap runs 60 cases: fourteen interpreted
NROs (engine suite 11), thirty-four kernel/NV API scenarios (NV suite 2), three
GPU-core cases (suite 2), and nine Vulkan checks (workload 4). Summary/export
stay together and older workload caches are invalidated.

The new color-clear NRO executes 5571 instructions/25 IPC/2 copies/18 A/11 B/
603 descriptor bytes and closes all handles/transfer objects/loans. Every pixel
matches the independent image formula. The additional sixteen NV scenarios
exercise B197 state, masks/rectangles, rights, padding, rejection, reply rollback,
aliases, context selection and lifetime. They call actual kernel APIs without
interpreting guest CPU bytes. See [the graphics batch](color-clear-batch.md).

A fourth value-only export, emulator_guest_clear_replay_v1(index), returns a
1264-byte/four-byte-aligned clear-plan/image value, with checked offsets and no
pointers. It is consumed by C++ inside the Vulkan group. Case 0 runs the NRO
and replays its actual command bytes through the original decoder. Other cases
replay owned BGRA/masked fixtures. Fixed host shaders lower clear operations;
there is no guest shader translation, draw support or live display handoff.
The existing three EngineReport exports retain ABI 2.

The [supplied 0.15 phone report](validation/g720-nv-channel-batch.md) confirms
all forty prior ARM64/G720 cases. This expanded batch awaits phone evidence.
Broader rendering, async synchronization, commercial startup and Z-A remain open.
Historical software/physical records remain under docs/validation/.

After the owned AVD boots, verify the entire flow with:

```sh
python3 tools/qa_android_batch.py --apk android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk --output reports/android-qa/v0.16.0-dev --version 0.16.0-dev --workload 16 --previous-workload 15 --gpu-tests 9 --gpu-workload 4 --engine-reference reports/v0.16-host-engine.json --nv-reference reports/v0.16-host-nv.json --core-reference reports/v0.16-host-gpu-core.json --shutdown
```

Generate references with the reference_report example (default, nv, gpu-core).
The tool compares all Rust fields except architecture, checks every Vulkan
outcome and summary, then verifies byte-exact export/cold restore and logs.
Upgrade requires a cached successful prior-workload report. --resume preserves
upgrade evidence while revalidating an existing successful current batch.
