# Original kernel and IPC subset

This is the implemented single-process reference path, not a complete Horizon
kernel or service runtime. All emulation logic is original. Public wire/ABI
references describe interfaces; no existing emulator core is a dependency.

## Guest ABI and limits

The main thread maps an owned 4 KiB TLS page at `0x7ffe0000`. MRS TPIDRRO_EL0
returns its guest TLR at `0x7ffe0200`, with a 256-byte message buffer. Additional
threads have separate TLS pages. Other system-register reads remain unsupported.
The initial main-thread handle is `0x100`; at most 256 handles may be live.
Closed IDs are not reused, so a stale ID cannot alias a newly created object.
ExitProcess releases all process handles and retains bounded debug/IPC traces
for the host's diagnostic evidence.

| SVC | Implemented behavior |
| --- | --- |
| `0x07` ExitProcess | End the process and release handles |
| `0x08` CreateThread | Validate entry/stack/priority/core, allocate owned TLS and a suspended thread |
| `0x09` StartThread | Make a created thread runnable |
| `0x0a` ExitThread | Terminate the current thread and signal joins |
| `0x0b` SleepThread | Zero yields; positive ns sleep using reference virtual time |
| `0x11` SignalEvent | Set a writable event's shared signal |
| `0x12` ClearEvent | Clear through a writable or readable event handle |
| `0x16` CloseHandle | Release one handle; aliases retain their object |
| `0x17` ResetSignal | Clear through a readable handle; unsignaled returns invalid state |
| `0x18` WaitSynchronization | Validate up to 64 event/thread handles; immediate results, polling or blocking waits with virtual timeouts |
| `0x19` CancelSynchronization | Interrupt a thread's active wait or retain a pending cancellation |
| `0x1f` ConnectToNamedPort | Bounded NUL-terminated ASCII name from X1; return result/handle in W0/W1 |
| `0x21` SendSyncRequest | Request/reply through guest TLS; W0 is the session handle on entry and result on return |
| `0x27` OutputDebugString | Preserve bounded raw guest debug bytes |
| `0x45` CreateEvent | Return distinct writable/readable handles in W1/W2 |

Events are manual reset. Polling does not consume a signal. Both capabilities
own the same signal state, so closing one does not invalidate the other.
Waits validate the complete handle list before selecting a signaled object.
A successful wait updates W1 with the selected index; error/timeout paths
preserve it. A zero-count wait never dereferences its array pointer. A blocking
wait retains the object references and resumes on signal, thread exit or a
virtual timeout. There are no host sleeps. See [scheduling.md](scheduling.md)
for the supported single-core scheduler, clock model and bounded blocked state.

Implemented guest results include invalid handle `0xe401`, invalid address
`0xe601`, timeout `0xea01`, interruption `0xec01`, too many wait handles `0xee01`, and invalid event
state `0xfa01`. Handle-table exhaustion, unsupported services/commands and
malformed IPC currently produce typed host faults. Complete Horizon error
mapping and exception delivery are not implemented.

## HIPC/CMIF v0

`crates/ipc` validates requests independently of service behavior. Supported
request types are CMIF Request (4) and Control (5), with version 0, zero context,
bounded raw data, 16-byte CMIF alignment/padding, and optional sender PID.
An empty special header is valid. The kernel overwrites a requested sender
identity with its owned guest PID 1; a guest cannot supply a host PID.

Descriptors, receive buffers, incoming copy/move handles, domains, context
variants, TIPC and other layouts are rejected explicitly. Replies are complete
zero-initialized 256-byte buffers with SFCO, result/data and up to 15 outgoing
moved session handles. Unaligned/oversized payloads are rejected.

A normal request runs on an isolated service snapshot. Before publishing any
mutation, the kernel checks guest reply write access, output bounds and handle
capacity. After writing the complete reply it commits handles and the candidate
service state. The current executor is single threaded, with no callbacks or
mapping changes during this transaction. Future services must honor the
`clone_for_request` contract and isolate mutable request state.

CMIF control command 2 (`CloneCurrentObject`) publishes another handle to the
same session identity. Requests through either handle update shared service
state; closing the original does not destroy the clone's state. This is
separate from cloning a service snapshot for transactional request handling.

`crates/services` implements only SM Initialize (command 0, sender identity and
8-byte payload required) and GetService (command 1, exactly 8 name bytes).
The only registered service is our emulator-owned `emu:test`: Add (0) checks
u64 overflow and returns the new counter; Read (1) returns its value. Each
GetService creates a fresh counter. This is a diagnostic service, not a stub
for a Nintendo device service. Unsupported operations never fabricate success.

## Evidence

The assembly sources are our own and assembled independently using LLVM 19.
Run each with `cargo run --offline -p emulator-cli -- run-nro <file>`.

| NRO | Expected result | Instructions | Successful IPC calls |
| --- | --- | --- | --- |
| `hello.nro` | Known debug string and process exit | 5 | 0 |
| `ipc-counter.nro` | SM initialization, Add 20/22, Read 42, close/double-close behavior | 384 | 5 |
| `ipc-clone.nro` | Shared counter reaches 43 after original handle closes | 529 | 7 |
| `events.nro` | Signaling, repeated polls, reset/clear, capability/close errors; seven verified outcomes | 87 | 0 |
| `threads.nro` | Separate contexts/TLS, sleeping child, blocking event wait, join and finite timeout | 97 | 0 |
| `cancellation.nro` | Deferred and active cancellation, coalescing, preserved indices, event state and readable clear | 106 | 0 |

Tests also cover truncated wire prefixes, PID replacement, malformed layouts,
reply capacity, read-only output, rollback after oversized replies, atomic
two-handle allocation at capacity, stale IDs, unknown commands, wait bounds,
all-handle validation and the bounded scheduling/wait corpus documented above.
This satisfies the initial owned single-core scheduling fixture checkpoint;
general Horizon synchronization and commercial startup remain incomplete.

Public interface references: [HIPC](https://switchbrew.org/wiki/HIPC),
[SVC](https://switchbrew.org/wiki/SVC),
[thread-local region](https://switchbrew.org/wiki/Thread_Local_Region),
[libnx HIPC declarations](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/sf/hipc.h)
and [CMIF declarations](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/sf/cmif.h).
