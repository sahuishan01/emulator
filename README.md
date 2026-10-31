# Original emulator

An emulator written from scratch, starting with Switch 1 and targeting Android
on the Realme GT 7 Dream Edition / Immortalis-G720. Pokémon Legends: Z-A is a
long-term compatibility target.

The project plan is split into [architecture](docs/architecture.md),
[runtime design](docs/design.md), and
[implementation milestones](docs/implementation-plan.md). These distinguish
the implemented components from the remaining Switch/Android work.

The original Rust engine loads six of our own static NRO programs and executes
them through an A64 interpreter subset. They exercise debug output, service
discovery/IPC, shared cloned-session lifetimes, events, thread scheduling,
blocking waits, joins, virtual timeouts and cancellation.
The Android lab embeds this engine, three GPU memory/command reference cases and six
native Vulkan checks: image transfers, depth testing, compute-to-draw visibility,
BC1 conversion, sampled draws and converted-image updates. All six GPU checks passed in the user's supplied
physical G720 0.3.0-dev report, including converted-texture sampling and updates
without native BC support. See the
[device evidence and its limits](docs/validation/g720-sampling.md).

The engine does not run commercial Switch games. Broader GPU method execution, shader
translation, broader synchronization/services, most CPU instructions and the gameplay application
are still to be implemented. The GPU Lab uses our native host shaders; it does
not execute guest graphics or run Z-A.
An original [GPU framing parser](docs/gpu-command-framing.md) now decodes bounded
GPFIFO entries and method packets. Shared CPU/GPU backing, GPU virtual mappings
and immutable decoded command batches now preserve ownership across unmaps and
address reuse. See [GPU memory and submission ownership](docs/gpu-memory-submission.md).
[Original DMA method execution](docs/gpu-dma-methods.md) now performs bounded
linear copies with per-subchannel state and transactional validation. NV service
submission, shaders and rendering remain separate work.

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
Handles, bounded HIPC/CMIF v0, session cloning and manual-reset events are also
implemented. Run `ipc-counter.nro`, `ipc-clone.nro`, `events.nro`, `threads.nro`
or `cancellation.nro` through the
same command to exercise them. See [the kernel/IPC subset](docs/kernel-ipc.md)
for exact behavior and limits. Unsupported operations produce explicit errors.
The [reference scheduler](docs/scheduling.md) switches guest contexts, preserves
per-thread TLS and advances virtual deadlines without sleeping the host.

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
| `crates/memory` | Shared owned backing, aliases/pinned views, resident budgets, transactional mappings and cross-region R/W/X checks |
| `crates/a64` | Registers, instruction decoding/execution, arithmetic flags, explicit faults and SVC events |
| `crates/loader` | Bounded static NRO validation, segments, permissions and BSS |
| `crates/ipc` | Bounded HIPC/CMIF v0 request parsing and reply encoding |
| `crates/kernel` | SVCs, handles, IPC, cloned sessions, threads, waits, deadlines and cancellation |
| `crates/services` | Minimal CMIF service manager and our `emu:test` diagnostic counter |
| `crates/session` | Process entry state, guarded stack, guest TLS and resumable execution budgets |
| `crates/texture` | Original bounded BC1 RGBA UNORM reference decoder |
| `crates/gpu` | GPU virtual mappings, pinned resources, bounded GPFIFO/Maxwell framing and owned decoded batches and linear DMA method execution |
| `crates/cli` | NRO/raw-program execution and demo |
| `crates/android-ffi` | Bounded C reports for six guest programs and three GPU memory/command cases |
| `examples/a64` | Our assembly sources and LLVM-generated test binaries |
| `examples/homebrew` | Six original homebrew sources and assembled NRO fixtures |
| `native/gpu-lab` | Original C++ Vulkan workloads, desktop CLI and Android JNI boundary |
| `android/gpu-lab` | Original engine and native GPU diagnostics with retained JSON/export |
| `tools/collect_android_gpu.py` | Optional read-only Android capability/exit-history capture |
| `docs/architecture.md` | Original emulator design and staged milestones |

Implemented instruction forms: NOP; ADR/ADRP; MOVN/MOVZ/MOVK W/X; immediate ADD/SUB with optional
flags; B/BL; B.cond; CBZ/CBNZ W/X; RET; unsigned-immediate LDR/STR W/X; MRS TPIDRRO_EL0; SVC.
Other forms fail explicitly. One host executor schedules guest threads; it does not
implement the full A64 ISA, floating point/SIMD, atomics, privileged exceptions,
or a page-table/MMU model. Memory accesses may cross adjacent mappings when
every part permits the operation; failed writes leave the entire range intact.

## Android engine and GPU lab

The verified development APK, checksum and packaging report are attached to
the [development prerelease](https://github.com/sahuishan01/emulator/releases/tag/v0.8.0-dev)
in this private repository. The APK runs original guest reference programs and host Vulkan diagnostics; it does not
run Switch games.

- [Download APK](https://github.com/sahuishan01/emulator/releases/download/v0.8.0-dev/emulator-gpu-lab.apk)
- [Download ZIP fallback](https://github.com/sahuishan01/emulator/releases/download/v0.8.0-dev/emulator-gpu-lab.zip), then extract and install the APK inside.

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
toolchain installed, also install the pinned Rust Android targets:

```sh
rustup toolchain install 1.85.1 --profile minimal --target aarch64-linux-android --target x86_64-linux-android
```

Then build (the Gradle task cross-compiles the Rust engine automatically):

```sh
cd android
./gradlew :gpu-lab:assembleDebug :gpu-lab:lintDebug --no-daemon
```

Output: `android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk`.
The APK includes arm64-v8a and x86_64 libraries. Install it, select **Run engine
and GPU checks**, then **Export report** to save its JSON through Android's file
picker. It uses the system Vulkan driver and requests no network or storage
permissions. Reports explicitly distinguish software renderers from physical
devices. Schema 3 separates six guest checks (`engine`), three GPU memory/command
cases (`gpuCore`) and six host Vulkan checks (`gpu`). See [the Android boundary](docs/android-engine.md)
for its build, report schema and verification limits.

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

The current suite has 102 Rust tests and 17 independently assembled fixtures
(11 raw A64 programs and six NROs).
Verify the BC1 fixture with `python3 tools/generate_bc1_fixture.py --check`.
All six Vulkan workloads passed on desktop llvmpipe with zero validation
errors and on an Android 15 SwiftShader AVD. A deliberately stale update in a
temporary control build fails the update test while the preceding five pass.
All six passed in the user's supplied physical G720 report, with validation disabled. APK
build, lint, signature, native architectures and 16 KiB ELF/ZIP alignment were
verified. In 0.4.0-dev, all four Rust guest checks also passed through JNI in
the Android AVD; upgrade invalidation, exact JSON export and cold restoration
passed. The [user-supplied 0.4.0-dev phone report](docs/validation/g720-engine.md)
also passes all four previous engine cases on ARM64. The later
[0.5.0-dev phone report](docs/validation/g720-scheduling.md) passes the two-thread
scheduling case as well. The [0.6.0-dev phone report](docs/validation/g720-cancellation.md)
also passes cancellation, closing all six guest-program device checkpoints.
The [0.7.0-dev phone report](docs/validation/g720-gpu-ownership.md) also passes
both GPU-core ownership cases. The new DMA execution case needs its own phone report.
The [0.5.0-dev Android AVD checkpoint](docs/validation/android-scheduling.md)
passes all five guest checks plus six Vulkan checks, with exact report export
and cold restoration.
The [0.6.0-dev AVD checkpoint](docs/validation/android-cancellation.md) adds
passing cancellation checks, for six guest programs plus six Vulkan workloads.
The [0.7.0-dev AVD checkpoint](docs/validation/android-gpu-ownership.md) passes
all fourteen cases, including GPU-core ownership, exact export and restoration.
The [0.8.0-dev checkpoint](docs/validation/android-dma.md) passes all fifteen
cases with original DMA execution, exact export and cold restoration.
These results cover the owned workloads; guest graphics, sustained
performance and 16 KiB Android runtime behavior remain untested.
Current state and next steps are recorded in [HANDOFF.md](HANDOFF.md).
