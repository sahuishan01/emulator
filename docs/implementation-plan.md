# Implementation plan

Plan baseline: 2026-10-30. This is delivery planning for the original emulator,
not a promise that commercial game compatibility follows a fixed schedule.
See [architecture.md](architecture.md) and [design.md](design.md) for boundaries
and contracts. Existing emulator settings changes and fork patches are outside
the current scope.

## Objectives and evidence

Primary platform: Switch 1 user-mode emulation on arm64 Android, with Realme
GT 7 Dream Edition / Immortalis-G720 as the first physical graphics target.
The eventual title target is the original Switch version of Pokémon Legends:
Z-A. Success claims must name the build, device, driver, title version and scene.

Current foundation: seven original Rust crates implement bounded guest memory,
a small A64 interpreter, a static NRO subset, debug-output/exit SVCs and a CLI
session. Our NRO prints its known message and exits after five instructions.
All 40 Rust tests and 11 independent LLVM fixture comparisons pass. The texture
crate adds original BC1 RGBA UNORM reference decoding and a reproducible fixture.

The separate Android GPU Lab APK builds and passes lint, signature and native
packaging checks. Six host workloads pass on llvmpipe with zero Vulkan
validation errors and on an Android 15 SwiftShader AVD. The original four, including
BC1 conversion, passed in the user's supplied physical G720 0.2.0-dev report.
This closes the conversion corpus checkpoint, not texture sampling or guest
graphics. APK export/state retention are exercised
separately from graphics;
there is no guest GPU, shader translator, scheduler/IPC or commercial game
support yet.

## Milestones and dependency gates

| ID | Deliverable | Dependencies | Exit evidence |
| --- | --- | --- | --- |
| M0 | CPU/memory seed | None | Existing microprogram demo and tests; this seed is complete, full ISA is not |
| M1A | Original NRO loading and process memory | M0 | Valid owned fixture maps with permissions/BSS; malformed corpus rejected without partial mapping |
| M1B | Android G720 Vulkan laboratory | Can begin now, independently of M1A | Capability report plus correct native clear/triangle, texture copy and compute visibility readbacks on the actual phone |
| M2 | Minimal guest execution through kernel interfaces | M1A plus needed ISA forms | Our NRO prints a known debug string and exits through implemented guest SVC behavior |
| M3 | Threads, handles and IPC slice | M2 | Owned multithreaded program exercises events, waits, timeouts and service requests with deterministic expected results |
| M4 | Guest GPU and shader vertical slice | M1B; M3 for end-to-end guest service submission | Our guest graphics program submits real guest commands and presents a known triangle through our original shader path |
| M5 | Required 3D/compute/texture correctness | M4 | Textured geometry, depth/stencil, copies, compute-to-draw, formats and aliases pass a reduced workload corpus on G720 |
| M6 | Broader executable/services and usable application | M3/M5 in relevant portions | Needed module/content paths, filesystem/saves, audio, input and lifecycle work for selected homebrew workloads |
| M7 | Performance and compatibility iteration | Correct reference paths from M2-M6 | JIT/CPU improvements and GPU optimizations preserve results; selected titles run through tested scenes |
| M8 | Z-A acceptance campaign | The actual required M6/M7 features | Reproducible progression into 3D gameplay with correct graphics, saves, stability and measured performance on G720 |

M1A and the M2 fixture gate are implemented for the explicitly restricted
[NRO/ABI subset](nro-loader.md). They do not establish general NRO/libnx or
commercial startup compatibility. M1B has a buildable laboratory and initial
offscreen workloads and an initial physical-device report. Its expanded
texture, shader-stage and presentation experiments below remain open. BC1
reference/GPU conversion is implemented and its initial physical corpus passed.
The next checkpoint is sampling the converted image in a draw, with reference
readback, synchronization, update/lifetime and resource-peak evidence.
The 0.3.0-dev native suite now implements sampled draws and changed-content
reuse in one submission, with a CPU reference and Vulkan allocation counters.
Its two new physical results remain pending. These are host workloads, not M4.
M3 onward is
unimplemented.

M1B is deliberately early. The project must learn what the real Mali driver can
do before assuming a shader or texture strategy. A native Android triangle
does not satisfy M4; reaching a game menu does not satisfy M8.

## First implementation batch: a real guest executable

1. Extend `memory` with a process mapping transaction and checked segment
   permissions/BSS initialization. Add cross-boundary and rollback tests;
   keep guest and host page granularity independent.
2. Add `crates/loader` with bounded readers, NRO validation and a `ProcessImage`
   description. Reject truncated headers, unsupported versions, overflowing
   offsets/sizes, incompatible overlaps and unreasonable allocation requests.
3. Create our own minimal homebrew NRO source and assembly/build script.
   Define entry state, stack and homebrew ABI requirements explicitly. This
   fixture must not depend on an emulator implementation or bundled console
   content.
4. Add only the A64 forms needed by that fixture and its startup code, with
   register/flags/fault tests. Extend to addressing, logical, pair and indirect
   branch forms as demanded by the next fixture rather than claiming full ISA.
5. Add the initial `kernel` process/context and SVC dispatcher. Implement the
   documented debug-output and process-exit behavior used by the fixture;
   validate guest buffers and preserve meaningful error outcomes.
6. Add `run-nro` to the CLI through an original session path. Raw execution
   remains available for CPU regression tests.

Done means a loader test suite, failure corpus, repeatable guest debug output,
observed process exit and a passing existing CPU/memory suite. ExitProcess has
no guest exit-status argument; do not invent one. It does not mean a
general commercial title can boot.

## First graphics batch: establish G720 feasibility

1. Add the minimum Android GPU-lab application and native boundary, with a
   reproducible arm64 build. Pin toolchain/dependency versions; verify native
   library packaging on 4 KiB and 16 KiB configurations.
2. Query actual device/driver, features, limits, queues and format/image usage
   support. Distinguish a physical GPU from a software renderer. Export a
   machine-readable report without changing the device driver.
3. Render a clear and fixed triangle, then textured/depth-tested geometry.
   Validate offscreen readbacks against owned reference data as well as the
   visible surface. Presenting one image alone does not validate all formats.
4. Exercise transfer-to-sample, compute-write-to-draw and render-to-readback
   ordering using small isolated workloads. Run synchronization validation in
   development and match outputs in non-validation builds too.
5. Prototype one exact texture fallback using an original reference decoder,
   then its GPU equivalent. Compare results, temporary allocations and memory
   peaks. This is an experiment, not permission to assume every compressed
   format can share the same path.
6. Record missing shader stages/operations and resource limits. Build a reduced
   workload for each candidate fallback. If a necessary path is unsupported,
   document the lowering/performance research required before promising it.

Done means a report and reproducible workloads on the user's actual G720,
including build/driver IDs and readback results. Desktop Vulkan tests can
validate host-independent logic but cannot replace this evidence. Until the
phone is available, parser, IR and CPU work can continue independently.

## Guest graphics integration batch

Implement the minimum service/driver-facing operations needed for an owned
guest graphics sample: GPU memory objects and mappings, channel/submission,
synchronization and display-buffer handoff. Add bounded command-list/method
decoding, initially only for the operations the sample actually emits.

Implement the original shader decoder and typed IR for that sample's vertex
and fragment stages. Validate generated SPIR-V and bind the resulting resources
through the already tested host backend. Keep self-authored command-buffer
fixtures available so GPU failures can be replayed without running the CPU or
recreating every guest service.

Required proof is the whole guest path: executable -> guest services -> guest
commands/shaders -> host renderer -> image. A host shader or manually injected
draw that bypasses guest decoding cannot satisfy this milestone.

## Incremental completeness

After the triangle, grow the workload corpus in explicit slices: indexed and
instanced draws; descriptors and vertex layouts; depth/stencil/blending;
mips/arrays/cube/volume textures; copies and aliases; compute; indirect work;
required geometry/tessellation/subgroup fallbacks; shader numerical behavior.
Prioritize according to reduced guest workloads, not assumptions about the
Z-A failure reported in another emulator.

Expand kernel/services and content loading alongside the workloads that need
them. Add shared memory, atomics/exclusive behavior, timers, service sessions,
filesystem flushes, audio and input with their own expected traces. Full
application startup, module linking, system/applet behavior and commercial
content requirements are substantive milestones, not configuration switches.

Implement ordinary in-game saves and export/import before save states. Defer
rewind, netplay, frame generation, large shader packs, cloud sync, custom driver
distribution and additional console engines until core correctness is proven.

## Performance work

Retain interpreter/reference-decoder paths. Profile first, then choose the
next optimization: hot-block CPU translation, memory fast paths, shader/pipeline
caching, batching, avoiding readbacks, conversion reuse or reduced temporary
memory. Keep changes independently reversible and compare identical workloads.

A JIT or native-execution backend is not a prerequisite to discover host GPU
limitations, but practical heavy games will require substantial CPU and GPU
performance work. There is no promise that every missing feature can be
emulated at playable speed on every Mali generation.

Track time to first frame, shader-compile stalls, frame-time distribution,
guest CPU time, GPU time where measurable, memory peaks and sustained thermal
behavior. Set title/device performance targets only after a correct baseline
exists. Generated frames do not count toward guest emulation throughput.

## Test and build workflow

For Rust changes run scoped tests, the workspace suite where cross-module
behavior changes, formatting and Clippy. Regenerate/verify assembly fixtures
when their source changes. Fuzz original executable, IPC, GPU command and shader
parsers as they are introduced. Assert failure is bounded and does not partially
commit invalid guest state.

Separate CI levels: ordinary host unit/integration tests; optional software
Vulkan correctness workloads; physical Android/Mali runs. Keep toolchain locks,
fixture hashes, workload versions and expected outputs. Do not label a test
skipped for unavailable hardware as passing on that hardware.

Development APKs use a distinct package ID and debug-export path. Stable saves
and test data have separate directories. A release requires real device evidence
for the advertised capabilities, installation verification and artifact hashes.
An APK compiling successfully does not establish game compatibility.

## Z-A acceptance record

When the emulator is ready for commercial compatibility work, record the
original Switch edition, title build/update, content requirements, emulator
commit, phone/Android/GPU/driver, settings and cache state. Test repeatable
menu-to-gameplay transitions, camera movement, scene changes, saves/reloads,
suspend/resume and a sustained session. Compare visible output and behavior,
not just whether the process remains alive or an FPS overlay advances.

If a failure occurs, isolate its first responsible layer and reduce it to an
owned workload where possible. Track unsupported services/opcodes separately
from shader errors, resource lifetime, driver failures and memory exhaustion.
The current reported black screen remains a target symptom, not a diagnosed
defect that this new code has already inherited or fixed.

## Main risks and planning policy

| Risk | Early action | Effect on delivery |
| --- | --- | --- |
| Incomplete guest GPU/OS research | Owned microtests and explicit evidence/provenance notes | Some features remain research work rather than predictable coding tasks |
| Mali feature gaps or driver defects | Early G720 lab and reduced fallback workloads | May require costly emulation or narrow the supported workloads/devices |
| Conversion and staging memory growth | Track all memory categories and peak working sets | A correct path may need memory optimization before a large title is viable |
| CPU/service/timing coverage | Homebrew slices and deterministic traces | A renderer alone cannot boot and run Z-A |
| Optimization changes semantics | Retained reference paths and hardware/vector comparisons | Performance changes cannot replace correctness gates |
| Limited hardware access | Keep host work independent; schedule physical-device checkpoints | Physical compatibility claims wait for actual runs |

Estimate each next bounded batch after the preceding gate reveals its missing
work. Do not assign a guaranteed Z-A completion date or convert lines of code,
test counts, a drawn triangle or a successful menu into a percentage of total
compatibility. A mature original Switch emulator is a major long-term effort;
the first near-term outcomes are the NRO/HLE and G720 feasibility slices.
