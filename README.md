# Original emulator

An emulator written from scratch, starting with Switch 1 and targeting Android
on the Realme GT 7 Dream Edition / Immortalis-G720. Pokémon Legends: Z-A is a
long-term compatibility target.

The project plan is split into [architecture](docs/architecture.md),
[runtime design](docs/design.md), and
[implementation milestones](docs/implementation-plan.md). These distinguish
the implemented components from the remaining Switch/Android work.

The original Rust engine now loads our own static NRO homebrew, executes it
through an A64 interpreter subset, emits debug output through an implemented
kernel call, and exits. A separate Android Vulkan GPU Lab APK exercises image
transfers, depth testing and compute-to-draw visibility. It has been tested on
software drivers; physical G720 testing is still required.

The engine does not run commercial Switch games. Guest GPU commands, shader
translation, threads/IPC, most CPU instructions and the gameplay application
are still to be implemented. The GPU Lab uses our native host shaders; it does
not execute guest graphics or run Z-A.

No existing emulator core is linked or copied into the implementation. Earlier
Eden/Lemon checkouts under `upstream/` are historical references, ignored by
Git and excluded from the Cargo workspace. Their recorded revisions are in
`docs/research/references.lock.json`. They are not runtime dependencies.

## Run our homebrew executable

Rust 1.85 or newer is sufficient; the Rust workspace has no external crate
dependencies and builds offline.

```sh
cargo run --offline -p emulator-cli -- run-nro examples/homebrew/hello.nro
```

Expected output:

```text
Hello from original Switch homebrew!
Guest process exited after 5 instructions.
```

The bounded loader validates the supported NRO0 header, maps RX/R/RW
segments, zeroes BSS and supplies a guarded stack and minimal homebrew entry
configuration. See [the exact supported format and ABI](docs/nro-loader.md).
Only debug-output and process-exit SVCs are implemented. Unsupported operations
produce explicit errors.

The raw CPU regression demo remains available:

```sh
cargo run --offline -p emulator-cli -- demo
```

The demo executes an independently assembled A64 program: a loop sums five
sevens, stores and reloads guest memory, calls a subroutine, and reaches an
SVC boundary with a result of 42 after 24 instructions. This raw harness
reports the SVC without kernel dispatch.

Run another raw little-endian A64 program, loaded at guest address `0x100000`:

```sh
cargo run --offline -p emulator-cli -- run-raw examples/a64/demo.bin --max-steps 100
```

The test runner maps a 4 KiB data area at `0x2000` and a 64 KiB stack at
`0x80000000`. This is a microprogram harness, not a Switch process layout.

## Structure

| Path | Implemented responsibility |
| --- | --- |
| `crates/memory` | Bounded owned mappings, reservation/commit/protect/unmap, transactional mapping and cross-region R/W/X checks |
| `crates/a64` | Registers, instruction decoding/execution, arithmetic flags, explicit faults and SVC events |
| `crates/loader` | Bounded static NRO validation, segments, permissions and BSS |
| `crates/kernel` | Debug output, process exit and minimal homebrew configuration |
| `crates/session` | Process entry state, guarded stack and resumable execution budgets |
| `crates/cli` | NRO/raw-program execution and demo |
| `examples/a64` | Our assembly sources and LLVM-generated test binaries |
| `examples/homebrew` | Our homebrew source and assembled NRO fixture |
| `native/gpu-lab` | Original C++ Vulkan workloads, desktop CLI and Android JNI boundary |
| `android/gpu-lab` | Native Android diagnostics application and JSON export |
| `tools/collect_android_gpu.py` | Optional read-only Android capability/exit-history capture |
| `docs/architecture.md` | Original emulator design and staged milestones |

Implemented instruction forms: NOP; ADR/ADRP; MOVN/MOVZ/MOVK W/X; immediate ADD/SUB with optional
flags; B/BL; B.cond; CBZ/CBNZ W/X; RET; unsigned-immediate LDR/STR W/X; SVC.
Other forms fail explicitly. The interpreter is single-threaded and does not
implement the full A64 ISA, floating point/SIMD, atomics, privileged exceptions,
or a page-table/MMU model. Memory accesses may cross adjacent mappings when
every part permits the operation; failed writes leave the entire range intact.

## Android GPU Lab

The verified development APK, checksum and packaging report are attached to
the [GPU Lab development prerelease](https://github.com/sahuishan01/emulator/releases/tag/v0.1.0-dev)
in this private repository. The APK is a host Vulkan diagnostic; it does not
run Switch games.

- [Download APK](https://github.com/sahuishan01/emulator/releases/download/v0.1.0-dev/emulator-gpu-lab.apk)
- [Download ZIP fallback](https://github.com/sahuishan01/emulator/releases/download/v0.1.0-dev/emulator-gpu-lab.zip), then extract and install the APK inside.

Open these links in a browser signed into the GitHub account with access to
this private repository. GitHub redirects downloads to temporary signed asset
URLs. If a saved download fails, start again from one of the permanent links
above rather than retrying its old `release-assets.githubusercontent.com` URL.
The ZIP contains the same APK, checksum, verification report and installation
notes. If both downloads fail only on the phone, the browser/network path to
GitHub's asset host still needs investigation; a server-side pass cannot prove
that path works.

Build instructions and the exact workloads are in
[native/gpu-lab/README.md](native/gpu-lab/README.md). With the pinned Android
toolchain installed:

```sh
cd android
./gradlew :gpu-lab:assembleDebug :gpu-lab:lintDebug --no-daemon
```

Output: `android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk`.
The APK includes arm64-v8a and x86_64 libraries. Install it, select **Run GPU
checks**, then **Export report** to save its JSON through Android's file
picker. It uses the system Vulkan driver and requests no network or storage
permissions. Reports explicitly distinguish software renderers from physical
devices. The APK is a development diagnostic, separate from the Rust runner.

## Validation

```sh
cargo test --offline --workspace
cargo fmt --all -- --check
cargo clippy --offline --workspace --all-targets -- -D warnings
```

The binary fixtures are generated from our own `.S` sources using LLVM's
AArch64 assembler, independently of the interpreter decoder. To regenerate or
verify them, install LLVM 19's `llvm-mc`, `ld.lld` and `llvm-objcopy`:

```sh
python3 tools/assemble_a64_fixtures.py
python3 tools/assemble_a64_fixtures.py --check
```

The current suite has 34 Rust tests and 11 independently assembled fixtures.
The three Vulkan workloads passed on desktop llvmpipe with zero validation
errors and on an Android 15 AVD using SwiftShader. APK build, lint, signature,
native architectures and 16 KiB ELF/ZIP alignment were verified. Those results
do not establish physical G720 compatibility or 16 KiB Android runtime behavior.
Current state and next steps are recorded in [HANDOFF.md](HANDOFF.md).
