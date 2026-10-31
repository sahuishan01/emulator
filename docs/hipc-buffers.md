# Bounded HIPC A/B buffer grants

Version 0.10.0-dev implements normal-mode HIPC map-alias A (send/input) and
B (receive/output) descriptors in the original parser/kernel. The binary layout
follows the public [libnx HIPC declarations](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/sf/hipc.h).
The Rust implementation, tests and guest assembly are original. This adds a
standard IPC transport subset, not standard NV service compatibility.

## Wire and limits

The first HIPC word carries A/B counts in bits 20-23/24-27. Descriptors follow
the optional special/PID header, ordered A then B, and precede raw CMIF data.
Each descriptor has three little-endian words:

| Word | Fields |
| --- | --- |
| 0 | Size bits 0-31 |
| 1 | CPU address bits 0-31 |
| 2 | Mode bits 0-1; address bits 36-57 in bits 2-23; size bits 32-35 in bits 24-27; address bits 32-35 in bits 28-31 |

Parsing preserves all 58 address/36 size bits. Only mode 0 (normal) is accepted.
Modes 1/3 require unimplemented NonSecure/NonDevice semantics; mode 2 is invalid.
Up to four incoming copied process/transfer handles are added in 0.11.0-dev; see
[NV initialization](nv-initialization.md). X/C static buffers, W exchange buffers,
incoming moves/other copied types, domains/context/TIPC,
receive-list controls and buffers on CMIF Control remain explicit errors.
The existing complete raw-data/CMIF padding and 256-byte TLS bounds still apply.

At most four A/B descriptors are accepted per request. The kernel supports
positive lengths with at most 8 MiB aggregate logical bytes and 4096 aggregate
backing spans. Overlapping grants count toward those limits separately. Empty
ranges and larger budgets are explicitly rejected; zero-length optional buffers
are not yet modeled. CPU memory itself has at most 4096 mappings, so even a
rejected pin preparation remains bounded. No input payload copies of descriptor
size are made just to grant buffers.

## Permissions and publication

The kernel preflights every descriptor before service preparation. A grants a
read-only MemoryView; B grants a write-only view, even when the underlying CPU
mapping is read/write. The new WRITE permission supports output mappings without
requiring read access. A gap, uncommitted region or missing required permission
rejects the whole request. B ranges cannot alias any TLS reply backing, including
through another CPU virtual address. Grants retain fixed backing identities and
permissions; no raw pointer or borrowed backing storage escapes.

Services explicitly require their expected buffer counts. Default services and
legacy emu:gpu calls require zero, so unused descriptors are not silently ignored.
Prepared effects still publish only after reply/handle validation. Failure drops
all grants/effects without changing output, candidate state or IPC traces.
Successful records add send_buffers, receive_buffers and logical buffer_bytes;
Android reports them as sendBuffers, receiveBuffers and bufferBytes. Counts cover
only successful calls, not a console timing or completion model.

## Buffered DMA diagnostic

`emu:gpu` command 2 takes three u64 GPU addresses: command, source and destination.
It requires A0 commands (<=64 KiB, multiple of four), A1 source bytes (<=4 MiB),
A2 GPFIFO entries (<=2048 bytes, multiple of eight) and B0 destination (<=4 MiB).
All ranges must be positive; the shared kernel budget also applies. Source and
destination have distinct GPU maps with their descriptor rights. Commands cannot
read B or write A. The synchronous original DMA subset and four-u64 reply are
unchanged; each call starts a fresh queue/class state and increments the session
counter only after publication. See [the bridge contract](guest-gpu-bridge.md).

`gpu-buffers.nro` supplies real descriptors, submits seven method writes/one
8-byte DMA launch, checks the destination and reply counters inside the guest,
queries the counter, closes both sessions and prints its expected message. It
exits after 327 instructions/four IPC calls with zero handles. The successful
buffered call has three A/one B/80 logical bytes. Engine suite 5 adds this eighth
NRO; direct GPU-core and native Vulkan checks remain separate.

Tests include literal high-bit descriptor vectors, all truncated prefixes,
PID/empty-special-header alignment, modes/counts, write-only grants, attempted
A writes/B reads, late invalid buffers/commands, TLS aliases, budgets/retained
pin cleanup and actual interpreted guest execution. Standard NV Initialize now adopts checked process/transfer-memory objects. Its required
device/ioctl/object layer remains open. Static/exchange/optional-buffer variants follow
owned workload needs; shader/rendering milestones remain open.
