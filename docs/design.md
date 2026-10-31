# Runtime design

This is the proposed design for the original emulator. It supplements
[architecture.md](architecture.md); planned interfaces are not implemented APIs.

Current implemented slice: bounded region memory, static NRO loading, A64
interpreter execution, bounded handles/IPC, session cloning, guest scheduling,
event/thread waits, cancellation and virtual deadlines in one process. The Kotlin/JNI lab runs nine original Rust guest
programs, three GPU-core memory/command cases and six Vulkan workloads, with report
export. Broader synchronization, guest graphics and the gameplay
Android API remain planned. See [kernel-ipc.md](kernel-ipc.md) and
[android-engine.md](android-engine.md) for the implemented contracts.
The [GPU memory/submission layer](gpu-memory-submission.md) implements shared
backing, owned GPU mappings and decoded batch capture.
[DMA method execution](gpu-dma-methods.md) adds bounded linear copies separately
from NV services, shaders and rendering. The
[guest diagnostic bridge](guest-gpu-bridge.md) now routes an NRO through IPC to
prepared DMA effects, published only after kernel reply/handle validation.
Normal [HIPC A/B grants](hipc-buffers.md) add read-only inputs/write-only outputs
and a second DMA guest, with whole-request range/budget validation.
[Transfer-memory lending/copied objects](nv-initialization.md) now support the
NV Initialize resource subset and restore owner rights after final release.

## 1. Process, addresses and memory

Use distinct `GuestVa`, `GpuVa`, `BackingId`, `ProcessId`, `ThreadId` and
`GuestHandle` types. Guest addresses never become raw host pointers through an
unchecked cast. A process owns its virtual mappings, permissions, heap, stack,
TLS and module list. Mappings refer to shared backing objects so aliases have
one identity and ownership history.

Keep guest mapping granularity separate from host allocation granularity.
Native Android code must handle both 4 KiB and 16 KiB host page configurations;
do not hardcode the guest page size into host memory-protection calls.
[Android page-size requirements](https://developer.android.com/guide/practices/page-sizes)

The current region implementation is the reference path. Reservation, commit,
map/protect/unmap and cross-mapping accesses are implemented, including batch
mapping validation and complete write preflight before changing bytes. Address
arithmetic, segment overlap and BSS sizes use checked arithmetic and explicit
allocation limits. Protect/unmap retain shared backing slices; pinned views and
aliases keep stable identity/lifetime. Originating allocations remain charged
until their final reference drops, including GPU/queue pins. A page-based
allocator and concurrent renderer coherence remain future work.

Planned renderer coherence: CPU-only memory keeps a checked path. Shared
CPU/GPU ranges will track last CPU write generation, GPU producer and completion.
When a CPU read needs GPU-written data, resolve the producer and perform the
required readback/invalidation. Never hold the scheduler's handle-table lock
while waiting for the GPU. Guest memory visibility and Vulkan host memory
visibility are separate responsibilities.

## 2. Executable loading

The first implemented loader uses our own static NRO homebrew fixture and a
restricted [format and ABI contract](nro-loader.md). The loader parses metadata,
validates the supported format version and file bounds, constructs a
`ProcessImage`, and requests mappings from the process address-space manager.
It does not mutate memory until validation succeeds. Initialize BSS, entry
state, stack and the homebrew entry ABI explicitly.

Separate executable parsing, process mapping, relocation/linking and content
mounting. Later NSO/module support adds its own metadata, decompression,
linking and guest environment requirements. Loading a raw instruction buffer
at the CLI's fixed address is not a substitute for any of those steps.
The NRO format describes text, read-only and data segments plus BSS.
[Public NRO format research](https://www.switchbrew.org/wiki/NRO)

Expose user content through a `ContentProvider` abstraction. It supplies
seekable files and title/update/mount metadata. Container parsing and other
content prerequisites are a separate later workstream; no games, console keys
or firmware are bundled into the project.

## 3. CPU execution contract

Proposed interface:

```text
CpuBackend.run(thread_context, address_space, execution_budget) -> CpuExit
CpuExit = SupervisorCall | Fault | TimesliceExpired | Breakpoint | Stop
```

`ThreadContext` includes the architecturally required integer, FP/SIMD, flag,
thread-local and exception-facing state as those features are implemented.
Keep the initial interpreter as a readable reference. Extend instructions
from the next guest program's actual requirements; unknown encodings remain
explicit errors, never no-ops.

Specify PC behavior at each exit. Our current SVC event advances PC by four;
faulting instructions preserve prior state. Add register-width, SP/zero,
flags, alignment, memory-ordering, exclusive-monitor and FP-mode tests before
claiming the corresponding feature.

A recompiler can later consume an original CPU IR. Cache compiled blocks by
guest code identity and invalidate them on executable memory changes. Retain
interpreter-versus-JIT tests, but also use curated architecture vectors and
our own programs run on real AArch64 hardware: two backends sharing a decoder
can share the same bug. Native guest execution is a later research option,
not an assumed shortcut around services, memory or GPU emulation.

## 4. Kernel, services and guest time

The current subset has up to 16 lifetime threads, a 256-entry handle table and
owned per-thread TLS message buffers. CMIF requests execute on isolated service snapshots;
reply bounds, guest write permissions and returned handles are checked before
publishing state. Cloned handles share a session identity. Event/thread waits
validate every handle, retain object references and resume on signal or virtual
timeout. The [single-core reference scheduler](scheduling.md) implements a
bounded part of the following design; its clock model is not hardware timing.

The kernel owns processes, threads, handle lifetimes, synchronization objects,
memory operations, timeouts and SVC dispatch. Begin with one host CPU executor
and a deterministic guest scheduler. Track guest execution budgets and virtual
time; when no guest thread is runnable, advance to the next scheduled event.
Do not implement guest sleeps as a series of host-thread sleeps.

Guest IPC parsing is separate from service behavior. Validate message headers,
buffer descriptors, handle transfers, output sizes and session/domain state.
Implement the needed HIPC/CMIF behavior and add other protocol variants only
when required. Keep version-specific command schemas explicit. Guest services
cover, incrementally, service discovery, files, display, GPU submission, input,
audio, settings and required applet/application behavior.
[Public IPC research](https://switchbrew.org/wiki/IPC_Marshalling),
[service interfaces](https://switchbrew.org/wiki/Services_API)

Unknown commands return a structured unsupported-operation outcome with the
service, command and guest call context. Do not invent successful replies to
make a title proceed. A service implementation must provide the behavior its
tests establish, including error and timeout behavior.

## 5. Guest GPU and shader pipeline

Keep the stages independently testable:

```text
guest GPU memory mappings + submitted command lists
    -> command/method decoding and engine state
    -> ordered draws, dispatches, transfers and synchronization operations
    -> shader decode -> typed IR -> capability lowering -> SPIR-V
    -> Vulkan resources and pipelines -> presentation/readback
```

Implement the GPU submission/address-space and syncpoint behavior exposed
through the guest interfaces, not a direct translation of guest game-library
calls into host Vulkan calls. The public NV service research documents the
service/ioctl boundary used for this work.
[Public NV service research](https://www.switchbrew.org/wiki/NV_services)

GPU virtual addresses resolve to backing objects independently of CPU virtual
addresses. Command-list parsing and shader decoding are bounds-checked and
budgeted. Preserve submission ordering and resource references for the period
the guest semantics require, rather than eagerly copying every pointer or
retaining arbitrary mutable host slices across threads.

The original shader IR carries types, control flow, stage inputs/outputs,
memory effects, texture operations and required numerical semantics. Lower
guest operations to native host features or a tested emulation path. Geometry,
tessellation, subgroup and other unsupported operations require individual
feasibility work; they cannot all be solved by one generic compute switch.
Validate emitted SPIR-V and feature requirements before creating a pipeline.

Default correctness mode waits for a required shader/pipeline. Asynchronous
compilation may reduce UI blocking, but must not silently discard a required
draw. Record unsupported operations and pipeline failures with stable IDs.

## 6. Mali texture and capability policy

Create a `DeviceCapabilities` report from queried features, limits, queue
families, driver information and format/image support. Use Vulkan 1.1 as the
initial proposed backend baseline and detect optional features explicitly;
raise requirements only when measured workloads justify it. Extension names
alone do not establish their enabled feature bits or equivalent core support.

For each guest texture choose a representation using its format, signedness,
color space, dimensions, image type, usages and required operations. Query the
matching host format and image-creation support. Vulkan format support is
operation-specific rather than one universal yes/no property.
[Khronos formats](https://docs.vulkan.org/guide/latest/formats.html)

Representation order is a design policy, subject to measured correctness:

| Candidate | Use condition |
| --- | --- |
| Native compressed texture | Exact required format/usage supported and workload verified |
| Correct uncompressed representation | Native compressed format unavailable; decoder and sampling semantics validated |
| GPU conversion into that representation | Same results as the reference decoder with correct barriers |
| Optional recompression | Measured memory benefit and accepted image-quality behavior; not the initial correctness path |

Retain original guest blocks for aliases, raw copies and readbacks. Test
block-linear layout, partial blocks, row/slice pitches, mips, arrays, cube
faces and volume textures. Color and alpha, sRGB versus linear sampling,
signed normalized channels and HDR require distinct expected results; do not
route every unsupported format through RGBA8 or a lossy ASTC representation.

A cache entry is identified by backing/range, content generation, view and
representation. Derived resources can be rebuilt; guest-visible writes cannot
be discarded. Account for original backing, decoded textures, temporary
conversion buffers, staging allocations, frame targets and pipeline caches.
Use bounded pools and backpressure, and never evict an in-flight resource or
the only copy of newer guest data. Allocation policy follows measured device
headroom rather than assuming 16 GB of physical RAM is available to the app.

## 7. Vulkan ordering and threads

Initial host workers:

| Worker | Ownership |
| --- | --- |
| Android UI | Navigation, surface lifecycle and commands |
| Session/guest CPU executor | Guest scheduling, thread contexts and kernel transitions |
| GPU/render worker | Vulkan queues, command submission and resource retirement |
| Bounded shader workers | Pure translation and allowed pipeline preparation |
| Audio callback | Consume prepared samples; no allocation, file I/O or guest execution |
| File/import worker | Bounded content reads and library operations outside real-time callbacks |

Use one graphics/compute-capable queue first where supported. Represent host
completion as typed monotonic tokens, independent of guest syncpoint counters
and wrap behavior. Timeline semaphores are optional; provide a fence/binary
path when unavailable. Swapchain acquire/present still use their required
binary synchronization. If multiple queues are added later, use explicit
dependencies and per-queue completion tracking rather than assuming one
unordered global counter is safe.
[Khronos synchronization](https://docs.vulkan.org/guide/latest/synchronization.html),
[timeline sample and presentation limitation](https://docs.vulkan.org/samples/latest/samples/extensions/timeline_semaphore/README.html)

Track last readers/writers, image layouts and stage/access requirements for
each resource/subresource. Apply barriers for transfer-to-sample,
compute-to-draw, render-to-readback and host visibility. Retirement waits for
the last consumer's completion. Device-wide idle waits may help isolate a
bug, but are not the default runtime dependency model.

Do not wait for CPU work while holding a lock the CPU needs to satisfy a GPU
wait. Bounded queues must have explicit backpressure and cancellation rules.
Pause/stop operations rendezvous at defined boundaries and wake blocked work.

## 8. Android host, input, audio and persistence

The Kotlin host submits session commands through a small JNI interface.
Native code owns frame/audio buffers and the rendering surface reference for
its documented lifetime. Session handles are validated; callbacks cannot
target a destroyed activity. A surface loss is an attachment change, not an
implicit guest reset. Start with arm64 Android; keep desktop host tests.

The current synchronous CLI session states are Loaded, Running, Paused, Blocked,
Exited and Faulted; an exhausted instruction budget pauses it for a later run.
The planned full runtime adds creation/stop transitions and asynchronous
safe-point coordination. Android process death
cannot be assumed to allow a final save callback. Map guest filesystem
flush/commit semantics to durable host operations and use atomic host metadata
updates. Save states are deferred until all guest-visible state can be captured
coherently and versioned.

Input is timestamped/batched into the session and translated by the guest HID
service. The guest audio implementation supplies samples to a bounded native
ring; the host adapter handles device changes and clock drift. Oboe is the
chosen candidate for low-latency Android output.
[Android Oboe documentation](https://developer.android.com/games/sdk/oboe)

Library files use user-granted Android file access. Cache shader source/IR,
SPIR-V and host pipeline data separately. Keys include the corresponding guest
content, translator/lowering version, capability choices and driver identity;
driver-specific pipeline blobs additionally honor the reported cache UUID.
Reject stale/truncated entries and rebuild without touching saves.

## 9. Diagnostics and reproducibility

Each test record includes build ID, workload/content build hash, device/GPU,
Android/driver versions, capabilities, settings, cache state and reproduction
steps. Distinguish guest faults, unsupported services/opcodes, translation
errors, driver failures, device loss and host memory pressure.

Keep bounded rings of recent CPU exits, service calls, GPU commands,
shader/pipeline IDs and completion events. Export relevant records on a
failure. Frame counts measure presented guest work; generated/duplicate
frames cannot establish emulation correctness. Performance work uses frame
times, compile stalls, memory, readbacks and sustained thermals.

Driver workarounds are scoped by tested device/driver/capability combinations
and linked to a reduced workload. The current black-screen report from other
emulators is not itself proof of any specific missing feature in our design.
