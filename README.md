# Original emulator

An emulator written from scratch, starting with Switch 1 and targeting Android
on the Realme GT 7 Dream Edition / Immortalis-G720. Pokémon Legends: Z-A is a
long-term compatibility target.

The project plan is split into [architecture](docs/architecture.md),
[runtime design](docs/design.md), and
[implementation milestones](docs/implementation-plan.md). These distinguish
the implemented components from the remaining Switch/Android work.

The original Rust engine loads our own static NRO programs and executes
them through an A64 interpreter subset. They exercise debug output, service
discovery/IPC, shared cloned-session lifetimes, events, thread scheduling,
blocking waits, joins, virtual timeouts, cancellation and guest DMA submissions,
including normal HIPC input/output buffer grants, transfer-memory lending,
standard nvdrv Initialize, NVMap metadata/import ioctls NV GPU address-space mapping, channel submission and completion.
The Android app embeds this engine, eighteen guest checks, fifty NV API scenarios,
seventy-six GPU core cases, twenty-five shader cases and eighty-three Vulkan checks. The first six native checks cover image transfers, depth testing, compute-to-draw visibility,
BC1 conversion, sampled draws and converted-image updates. All six GPU checks passed in the user's supplied
physical G720 0.3.0-dev report, including converted-texture sampling and updates
without native BC support. See the
[device evidence and its limits](docs/validation/g720-sampling.md).

The engine does not run commercial Switch games. Broader GPU method execution, shader
interfaces, broader synchronization/services and most CPU instructions
are still to be implemented. The support diagnostics replay decoded color clears and translate a small original SM53 shader subset to SPIR-V.
Guest header/vertex/draw commands now feed a CPU reference renderer and Vulkan replay.
The [live homebrew demo](docs/live-homebrew.md) now runs our own Catch NRO with
Vulkan completion, Android presentation and touch input. It does not run Z-A.
An original [GPU framing parser](docs/gpu-command-framing.md) now decodes bounded
GPFIFO entries and method packets. Shared CPU/GPU backing, GPU virtual mappings
and immutable decoded command batches now preserve ownership across unmaps and
address reuse. See [GPU memory and submission ownership](docs/gpu-memory-submission.md).
[Original DMA method execution](docs/gpu-dma-methods.md) now performs bounded
linear copies with per-subchannel state and transactional validation.
An [emulator-owned guest bridge](docs/guest-gpu-bridge.md) now lets an original
NRO submit these commands through IPC. [HIPC A/B buffer descriptors](docs/hipc-buffers.md)
now carry granted inputs/outputs to a second guest DMA fixture.
[Transfer memory and nvdrv Initialize](docs/nv-initialization.md) now retain
copied process-owned arenas and restore CPU rights on final release.
[NV device descriptors and metadata ioctls](docs/nvmap-devices.md) now support
NVMap Create/Alloc/SIZE/ALIGNMENT/KIND/Free with retained guest backing.
[NV GPU address spaces](docs/nv-gpu-address-space.md) now map retained slices
with ownership across NVMap Free/device Close and CPU unmap/address reuse.
[NV channel submission and completion](docs/nv-channel-batch.md) now execute
original DMA, preserve channel state and retain backing after bound fds close.
[Guest color-clear and Vulkan replay](docs/color-clear-batch.md) now add B197
render targets, rectangles/masks, atomic row writes and pixel-exact replay.
The [shader translation batch](docs/shader-translation-batch.md) adds original typed IR
and SPIR-V. The [guest draw batch](docs/guest-draw-batch.md) connects SPH headers, vertex
fetch, B197 draw state, transactional rasterization and guest-command Vulkan replay.
The supplied [0.17 G720 report](docs/validation/g720-shader-translation-batch.md) passes
all 78 previous checks. The [0.18 G720 report](docs/validation/g720-guest-draw-batch.md)
passes all 102. The [0.19 live G720 report](docs/validation/g720-live-homebrew.md)
passes103 checks and records578 presented homebrew frames. Broader shaders, general
display services and commercial games remain open.

The supplied [0.21.1 G720 report](docs/validation/g720-texture-batch.md) confirms
all 121 checks and presentation of the updated texture pattern. The supplied [0.22.1 physical report](docs/validation/g720-one-tap.md) confirms
all 134 diagnostics and six visible patterns, including indexed geometry. The new
storage batch is now also [confirmed on the target Mali GPU](docs/validation/g720-texture-storage.md):
all 151 checks and twelve patterns pass, including BC1/BC3 without native BC support.
The [depth batch](docs/depth-batch.md) now builds on ordered memory snapshots
to compare and write a bounded D32F attachment. Nine new cases cover every
comparison and disabled depth writes. The [physical depth report](docs/validation/g720-depth.md)
now confirms all 168 checks and 21 patterns. The [0.25 interpolation extension](docs/sloped-depth-batch.md)
adds sloped depth, now [confirmed on physical Mali](docs/validation/g720-sloped-depth.md)
with 172 diagnostics and 23 patterns. The [0.26 depth-clear batch](docs/depth-clear-batch.md)
adds sixteen rendered scenarios and twenty portable correctness groups, bringing
the single run to 208 diagnostics and 39 patterns, now
[confirmed on physical Mali](docs/validation/g720-depth-clear.md). The
[0.27 projection batch](docs/projection-batch.md) adds division by positive W
across textures, indexed geometry and depth: 240 diagnostics and 55 patterns.
The [physical 0.27 report](docs/validation/g720-projection.md) passes 239/240 checks and 54/55 patterns; the projected inset texture fails exact pixel comparison. The 0.27.1 report reproduces the same failure. The [0.27.2 investigation](docs/inset-evidence.md) adds twelve controls and exact failure evidence; a physical repair is not yet confirmed.
Broader 3D/runtime and games remain open.

No existing emulator core is linked or copied into the implementation. Earlier
Eden/Lemon checkouts under `upstream/` are historical references, ignored by
Git and excluded from the Cargo workspace. Their recorded revisions are in
`docs/research/references.lock.json`. They are not runtime dependencies.

## Android app

The **0.27.3-dev** app opens a persistent, searchable homebrew library. It includes
Catch and a guest texture scene, imports supported static `.nro` files through Android's file picker, runs
their actual code with visible output/errors, and saves recents and player settings.
Tap **Test all & export JSON** on the home screen to display all 55 patterns,
run all 252 checks and automatically save the JSON in **Downloads/Emulator**.
The result offers **Share JSON**. See [one-tap testing](docs/one-tap-testing.md).
Manual diagnostics and report export remain under **Settings → Diagnostics**.
See [the app architecture, import contract and limits](docs/android-app.md).

Private development downloads (sign into GitHub with repository access):
[APK](https://github.com/sahuishan01/emulator/releases/download/v0.27.3-dev/emulator.apk),
[ZIP](https://github.com/sahuishan01/emulator/releases/download/v0.27.3-dev/emulator.zip),
[release page](https://github.com/sahuishan01/emulator/releases/tag/v0.27.3-dev).
The existing Android application ID/signing identity is retained for updates.
The [in-app updater](docs/app-updates.md) now checks private GitHub releases, verifies downloads and opens Android's installer. Open **Updates** in the library or **Settings → App updates**. Manual checks stay visible without a saved token; **Test GitHub access** shows the result and last successful check. Connect once using read-only repository access.
See the [projection profile](docs/projection-batch.md) and
[0.27 rendering validation](docs/validation/android-projection.md) and [updater validation](docs/validation/android-app-updates.md). See [0.27.2 investigation validation](docs/validation/android-inset-evidence.md) and [manual update controls validation](docs/validation/android-update-controls.md).
This is a debug development build, not a production-ready commercial-game emulator.
See [indexed geometry](docs/indexed-geometry-batch.md) and [storage implementation](docs/texture-storage-batch.md) and [build verification](docs/validation/android-texture-storage.md).

## Play our original homebrew

Install the development APK and tap **Play Catch** in your library. Drag to catch the
falling green square with the red paddle; hold Left/Right to keep moving, or
use Pause/Resume and Reset. The emulated NRO performs the game logic and GPU submissions. This
is a64x64 homebrew reference demo, with an emulator-owned input/display mailbox.
Commercial games, full Horizon display/HID and practical console performance
remain unsupported. See [the execution path and limits](docs/live-homebrew.md).

## View guest textures

Open **Texture scene → Open scene** in the library. **Next pattern** cycles through
RGBA, BGRA, updated textures, indexed geometry and six pitch/tiled/BC1/BC3
storage variants, eleven depth cases, sixteen depth-clear cases and sixteen projection cases. Native BC support is not required for the software fallback. This renders our captured guest shaders,
vertex coordinates and texture descriptors through Vulkan; it is not a prerecorded
image. The original guest tests pixels, rollback and resource lifetime. See the
[texture and varying profile](docs/texture-draw-batch.md) and [indexed draw profile](docs/indexed-geometry-batch.md).

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
for exact behavior and limits. `gpu-dma.nro` exercises the original guest-to-DMA
bridge; `gpu-buffers.nro` uses normal HIPC A/B input/output descriptors. Both
use `emu:gpu`, not the standard nvdrv ABI. Unsupported operations
produce explicit errors. `nv-initialize.nro` uses the standard Initialize RPC
and checks transfer-memory restoration. `nvmap.nro` opens a device, creates,
queries and frees an unallocated NVMap object through buffered ioctls. See
[the exact device subset](docs/nvmap-devices.md). `nvmap-alloc.nro` imports existing
backing, queries alignment and verifies the Free address/byte preservation. Fresh
allocation and full NV bootstrap remain open.
`nv-gpu-map.nro` tests mapping cleanup; `nv-channel-batch.nro` exercises the
combined channel/job/fence/rejection/lifetime path. `nv-color-clear.nro` adds
three-region pixels, rejected-job rollback and retained graphics bindings.
`nv-draw.nro` submits actual header-backed shader draws and validates rollback,
pixels, fences and closed-owner lifetime.
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
| `crates/ipc` | Bounded HIPC/CMIF v0, normal A/B descriptors, incoming copy IDs and reply encoding |
| `crates/kernel` | SVCs, transfer-memory lending, copied objects, IPC, cloned sessions, threads and waits |
| `crates/services` | SM, `emu:test`, `emu:gpu`, nvdrv Initialize, NVMap, GPU mapping, channels, DMA/color-clear submission and completion |
| `crates/session` | Process entry state, guarded stack, guest TLS and resumable execution budgets |
| `crates/texture` | Original bounded BC1 RGBA UNORM reference decoder |
| `crates/gpu` | GPU virtual mappings, pinned resources, bounded GPFIFO/Maxwell framing, owned decoded batches, DMA/clear/draw reference execution |
| `crates/shader` | Bounded raw SM53/SPH decoder, typed SSA reference and direct SPIR-V emitter |
| `crates/cli` | NRO/raw-program execution and demo |
| `crates/android-ffi` | Bounded diagnostics, live Catch and value-only imported-program upload/execution |
| `examples/a64` | Our assembly sources and LLVM-generated test binaries |
| `examples/homebrew` | Nineteen original homebrew sources and assembled NRO fixtures (eighteen reference cases plus Catch) |
| `native/gpu-lab` | Original C++ Vulkan workloads, desktop CLI and Android JNI boundary |
| `android/gpu-lab` | Main Android library/import/player/settings and support diagnostics/export |
| `tools/qa-documents` | Separate test-only SAF provider; never part of the main APK |
| `tools/collect_android_gpu.py` | Optional read-only Android capability/exit-history capture |
| `docs/architecture.md` | Original emulator design and staged milestones |

Implemented instruction forms: NOP; ADR/ADRP; MOVN/MOVZ/MOVK W/X; immediate and shifted-register ADD/SUB with optional
flags (LSL/LSR/ASR); B/BL; B.cond; CBZ/CBNZ W/X; RET; unsigned-immediate LDR/STR W/X; MRS TPIDRRO_EL0; SVC.
Other forms fail explicitly. One host executor schedules guest threads; it does not
implement the full A64 ISA, floating point/SIMD, atomics, privileged exceptions,
or a page-table/MMU model. Memory accesses may cross adjacent mappings when
every part permits the operation; failed writes leave the entire range intact.

## Build and diagnose the Android app

The verified development APK, checksum and packaging report are attached to
the [development prerelease](https://github.com/sahuishan01/emulator/releases/tag/v0.27.3-dev)
in this private repository. The APK runs original guest reference programs and host Vulkan diagnostics; it does not
run commercial Switch games.

- [Download APK](https://github.com/sahuishan01/emulator/releases/download/v0.27.3-dev/emulator.apk)
- [Download ZIP fallback](https://github.com/sahuishan01/emulator/releases/download/v0.27.3-dev/emulator.zip), then extract and install the APK inside.

Open these links in a browser signed into the GitHub account with access to
this private repository. A 404 can mean the browser is not signed in or lacks
access; [sign in and open the release](https://github.com/login?return_to=%2Fsahuishan01%2Femulator%2Freleases%2Ftag%2Fv0.26.0-dev)
in Chrome, then choose the APK under Assets. GitHub redirects downloads to temporary signed asset
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
The APK includes arm64-v8a and x86_64 libraries. For support, open **Settings → Diagnostics**, select **Run all checks**, then **Export report** to save its JSON through Android's file
picker. It uses the system Vulkan driver and requests no network or storage
permissions. Reports explicitly distinguish software renderers from physical
devices. Schema4/workload 23 separates seventeen guest checks (`engine`), eleven GPU-core
cases (`gpuCore`), fifty NV scenarios (`nv`), twenty-five shader cases (`shader`) and
nineteen Vulkan cases (`gpu`). All121 run together, with one summary and export. See [the Android boundary](docs/android-engine.md)
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

The current suite has 250 Rust tests and 31 independently assembled fixtures
(12 raw A64 programs and nineteen NROs). Verify guest draw pages with
`python3 tools/generate_draw_fixtures.py --check` and emitted shader modules with
`python3 tools/generate_shader_fixtures.py --check --validate-spirv`.
Verify texture/varying fixtures with `python3 tools/generate_texture_fixtures.py --check --validate-spirv`.
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
both GPU-core ownership cases. The [0.8.0-dev phone report](docs/validation/g720-dma.md)
also passes the original DMA case. The [0.9.0-dev phone report](docs/validation/g720-guest-dma.md)
also passes guest DMA submission. The [0.10.0-dev phone report](docs/validation/g720-hipc-buffers.md)
also passes buffered DMA. The [0.12.0-dev phone report](docs/validation/g720-nvmap.md)
also passes transfer-memory ownership and NVMap metadata, closing all nineteen
prior phone checkpoints. The [0.13.0-dev phone report](docs/validation/g720-nvmap-backing.md)
also passes retained backing imports and ABI 2, closing all twenty prior cases. The [0.14.0-dev report](docs/validation/g720-nv-gpu-map.md)
also passes mapping ownership, closing all twenty-one prior phone checks.
The [0.5.0-dev Android AVD checkpoint](docs/validation/android-scheduling.md)
passes all five guest checks plus six Vulkan checks, with exact report export
and cold restoration.
The [0.6.0-dev AVD checkpoint](docs/validation/android-cancellation.md) adds
passing cancellation checks, for six guest programs plus six Vulkan workloads.
The [0.7.0-dev AVD checkpoint](docs/validation/android-gpu-ownership.md) passes
all fourteen cases, including GPU-core ownership, exact export and restoration.
The [0.8.0-dev checkpoint](docs/validation/android-dma.md) passes all fifteen
cases with original DMA execution, exact export and cold restoration.
The [0.9.0-dev checkpoint](docs/validation/android-guest-dma.md) passes all
sixteen checks, including the new guest-to-DMA NRO, exact export and cold restoration.
The [0.10.0-dev checkpoint](docs/validation/android-hipc-buffers.md) passes all
seventeen cases, including normal HIPC A/B grants, exact export and cold restoration.
The [0.11.0-dev checkpoint](docs/validation/android-nv-initialization.md) passes all
eighteen cases, including copied transfer-memory ownership and NV Initialize,
exact export and cold restoration.
The [0.12.0-dev checkpoint](docs/validation/android-nvmap.md) passes all
nineteen cases, including NV descriptors and unallocated NVMap metadata ioctls,
exact export and cold restoration.
The [0.13.0-dev checkpoint](docs/validation/android-nvmap-backing.md) passes all
twenty cases, including retained NVMap backing imports and ABI 2, exact export
and cold restoration.
The [0.14.0-dev checkpoint](docs/validation/android-nv-gpu-map.md) passes all
twenty-one cases, including NV GPU mapping lifetime and rollback, exact export
and cold restoration after a test-device restart.
The [0.15.0-dev combined checkpoint](docs/validation/android-nv-channel-batch.md)
passes all forty cases with one tap, including channel/DMA/fence integration and
eighteen protocol rejection/rollback/lifetime cases. Export and cold restoration
are byte exact.
The [0.16 Android checkpoint](docs/validation/android-color-clear-batch.md)
passes all sixty checks, including the new guest clear image and Vulkan replay,
with byte-exact export and cold restoration.
The [0.15 phone checkpoint](docs/validation/g720-nv-channel-batch.md) confirms
all forty previous cases on the actual G720. The new color-clear batch awaits
phone evidence. These results cover the owned workloads; guest shaders/draws, sustained
performance and 16 KiB Android runtime behavior remain untested.
Current state and next steps are recorded in [HANDOFF.md](HANDOFF.md).
