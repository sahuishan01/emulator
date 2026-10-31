# Guest-to-DMA diagnostic bridge

Version 0.9.0-dev connects an original A64 NRO to the original DMA executor via
SVC, HIPC/CMIF and `emu:gpu`. This emulator-owned protocol is deliberately
separate from nvdrv. Version 0.10.0-dev adds normal
[A/B buffer grants](hipc-buffers.md) and buffered command 2. Version 0.11.0-dev
adds [transfer-memory/copied-handle NV initialization](nv-initialization.md).
NV device objects/ioctls, shaders and rendering remain open. No commercial
program is expected to use this diagnostic service.

## Protocol and bounds

After SM Initialize, GetService with the eight bytes `emu:gpu\0` creates a fresh
session. CMIF command 0 takes exactly eight little-endian u64 fields:

| Field | Meaning |
| --- | --- |
| 0, 1, 2 | CPU command address, GPU command address, command byte length |
| 3, 4, 5 | CPU data address, GPU data address, data byte length |
| 6, 7 | CPU address and byte length of the GPFIFO entry array |

Command bytes must be a positive multiple of four, at most 64 KiB. The data
range must be positive and at most 4 MiB; this diagnostic contract requires the
whole range readable/writable. Entry bytes must be a positive multiple of eight,
at most 2048 (256 entries). CPU pins require existing permissions; GPU maps are
40-bit, bounded and non-overlapping. Existing packet/queue/execution limits apply.
The GPU command stream contains actual method writes, not a high-level copy RPC.

Preparation creates one temporary GPU address space and queue. It imports
shared command/data views, captures entries and command bytes, decodes methods
and prepares the front batch. It preserves source/destination backing pins until
publication. Each call starts with fresh GPU class state; persistent channel/NV
objects are not modeled. Command 0 returns four u64 values: method writes, copy
launches, copied bytes and completed calls in this session. Command 1 accepts no
payload and returns the completed-call counter. Command 2 accepts standard A/B
descriptors and three GPU addresses; its exact transport/limits are documented
in [HIPC buffers](hipc-buffers.md). Cloned session handles share
that counter; each separate GetService starts at zero. Unsupported requests
produce typed host faults under the current kernel contract.

## Kernel publication contract

`Service::prepare` receives a context that can grant existing memory rights.
Trusted original services must keep preparation side-effect free, retain pinned
views and return an optional `PreparedServiceEffect`. A candidate service snapshot
isolates state changes. Each effect declares every backing span it may write;
the kernel rejects more than 4096 spans, empty/overflowing spans, and any overlap
with the complete TLS reply backing, including CPU aliases.

The kernel preflights reply access, reserves trace space, validates effect ranges,
stages outgoing handles and encodes the complete reply before committing an
effect. A late encoding/handle error drops the prepared effect without touching
guest data or live service state. Commit is synchronous, single-threaded and has
no kernel callbacks or mapping changes. Effects must not allocate, fail or write
outside their declared pins; this is a trusted core interface, not isolation
against a malicious service implementation. The reply is written through its
validated pin, then handles, candidate state and the IPC trace are published.
Internal invariant failures can panic; this is not crash isolation.

`SubmissionQueue::prepare_front` consumes a queue into `PreparedSubmission`.
Preparation failure consumes/drops that owned queue without memory effects;
use `execute_front` when failed fronts must remain available. Dropping a prepared
submission cancels it. `commit` executes once and returns the updated queue and
statistics. Data mappings are pinned at preparation; live bytes are read at
commit, in launch order. Address reuse cannot redirect retained pins.

## Executable evidence

Our LLVM-assembled `gpu-dma.nro` discovers the service, supplies commands/entries
from guest memory, submits two dependent 8-byte copies, verifies both CPU-visible
results are 42, checks reply counters, queries one completed call and closes
both sessions. It prints `Guest DMA copied 16 bytes; dependent copies verified.`
and exits after 338 instructions/four IPC calls, with zero handles. There are
10 method writes/two launches/16 copied bytes. Engine suite 4 adds this seventh
actual guest case. GPU-core suite 2's direct DMA case remains separate and has
11 writes because it uses another GPU alias; the two counts are intentionally
different.

Tests cover prepared cancel/commit, live source changes, exact byte results,
late unsupported commands, range/permission/budget failures, TLS aliases, and
state/counter rollback. Kernel tests attach actual prepared writes to oversized
reply and handle-exhaustion cases, proving those failures do not publish effects.

Next: extend buffer/handle variants and implement device/ioctl objects required by the
public [NV service interface](https://switchbrew.org/wiki/NV_services), then
route the supported standard ioctl/object subset to this executor. The
original guest-program-to-image/shader milestone remains open.
