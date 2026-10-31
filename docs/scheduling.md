# Original thread scheduling and blocking waits

Version 0.5.0-dev adds a deterministic single-core reference scheduler to the
original Rust kernel. It shares the process address space and handle table
between guest threads while preserving each thread's CPU context and TLS.
This is an implemented subset, not full Horizon scheduling or timing behavior.

## Creation and context

CreateThread (`0x08`) reads entry/argument/stack from X1/X2/X3 and priority/core
from W4/W5, returning W0 result and W1 thread handle. Entry must be aligned and
executable; the 16-byte-aligned stack must have a writable 16-byte range below
it. Priority 0..63 is supported, with smaller numbers running first. Only core
0 and -2 (default, resolved to 0) are supported. Other cores fail explicitly.

The thread starts in Created state. StartThread (`0x09`) makes it runnable;
starting it again returns invalid state. Its CPU starts with X0=argument,
SP=provided stack, PC=entry, zeroed other general registers/flags and its own
TPIDRRO_EL0. Entry, stack, priority, handle capacity and TLS mapping are checked
before publishing the new thread. Failed creation does not leak a handle,
consume a thread ID or leave a partial TLS mapping.

The prototype allows 16 total threads per process lifetime, including main and
exited records, and 256 live handles. Thread IDs/TLS slots are not recycled yet.
TLS page N is `0x7ffe0000 + N*4096`, with TLR at page+`0x200`. This bounded arena
ends before the entry-config mapping. Guest stacks are caller-provided; the
kernel does not allocate or infer their complete extent. Main's existing
guarded stack is unchanged.

## Scheduling and virtual time

One host executor interprets one guest instruction at a time. Runnable threads
are selected by priority, then FIFO ready order. Equal-priority threads rotate
after a 64-instruction quantum. A higher-priority runnable thread preempts at
the next instruction boundary. SleepThread (`0x0b`) with zero yields; a positive
duration blocks until its virtual deadline. Negative yield/migration variants,
multicore affinity, priority inheritance and dynamic-priority SVCs are unsupported.

Each executed instruction advances the reference clock by one virtual ns.
This is a deterministic model, not a measured console frequency or cycle
accuracy claim. When no thread can run, the clock jumps directly to the next
sleep/wait deadline, without host sleeps or unbounded polling. Deadlines and
clock increments use checked arithmetic; overflow faults explicitly. A signal
observed on the same reference boundary as a deadline wins over timeout.

Execution budgets count interpreted instructions across all threads. Pausing
and resuming with different budget sizes preserves ordering, final CPU/memory
results, instruction counts and scheduler statistics. If every active thread
waits indefinitely and no future deadline exists, Session returns `Blocked`
without declaring exit or spinning on the host. Resuming an unchanged blocked
session returns the same outcome without executing instructions. The CLI
reports this state as an explicit nonzero result.

## Wait and lifetime behavior

WaitSynchronization (`0x18`) validates the complete guest handle array once.
It supports readable events and terminated-thread signals. A signaled object
returns immediately; a zero timeout polls; a positive timeout blocks; a
negative timeout waits indefinitely. Errors and timeouts preserve X1; success
returns the selected array index there. Zero handles never dereference the
array pointer and may be used for a timed wait.

A blocked wait retains references to objects, independently of their handles
and the guest array bytes. Another thread can close a handle or overwrite the
array without invalidating the pending wait or redirecting it to another object.
Manual-reset events wake all existing waiters; clearing later does not reblock
those already awakened. Register/flag state and TLS remain per thread.

CancelSynchronization (`0x19`) accepts a real thread handle. It interrupts an
active synchronization wait with result `0xec01`, preserving X1 and dropping
the old deadline/object references. When a thread is not in such a wait, a
pending flag coalesces repeated requests. Valid immediate signaled results,
zero-timeout polls and argument errors leave that flag pending; the next wait
that would block consumes it once. SleepThread is not a synchronization wait:
it retains pending cancellation without waking early. Later signals/timeouts
cannot overwrite a completed cancellation. Other threads' flags are independent.
Pseudo handles and ReplyAndReceive remain outside this implemented subset.

ClearEvent accepts readable as well as writable event handles. SignalEvent
continues to require a writable handle; cancellation does not signal an event.

ExitThread (`0x0a`) marks its thread object permanently signaled for joins. A
running thread survives closing its handle. Exiting the last active thread
ends the process; ExitProcess (`0x07`) terminates all threads. Process exit
releases handles. TLS/record reclamation before process teardown is future
work. Mutexes, condition variables and address arbitration remain
explicitly unsupported; they do not fabricate successful synchronization.

## Evidence and next work

The original LLVM-assembled `threads.nro` creates a child, checks independent
registers/flags/TLS, sleeps and signals an event, joins while the child sleeps,
then verifies a finite wait timeout. It executes 97 instructions, creates two
threads, switches contexts four times, blocks on three waits, records one
timeout, and exits with value 42 and zero handles. Reference virtual time ends
at 1680 ns. These counts are the owned fixture oracle, not gameplay timing.

Tests also cover higher-priority preemption, equal-priority time slicing/yield,
timeout while another thread runs, multi-waiter wake/clear, closing a waited
handle, changed guest arrays, thread joins, creation rollback/limits, deadline
overflow, bounded deadlock reporting, and execution budgets of 1/7/63/64/65.
The Android engine suite adds this as its fifth real guest program and reports
per-case thread/switch/wait/timeout/virtual-time counters.

The new original `cancellation.nro` checks a deferred coalesced request, an
active blocked-wait interruption, preserved result indices, event state and
readable ClearEvent. It exits after 106 instructions, four switches, two
blocking waits and two delivered cancellations, at 1091 reference ns. The
Android suite adds this as its sixth actual guest program.

Next kernel work includes remaining synchronization/ISA requirements for GPU
service submission. [GPU command framing](gpu-command-framing.md) is now
implemented separately, together with owned GPU memory/queues and bounded
[linear DMA execution](gpu-dma-methods.md). NV-service submissions, broader
method execution, shader translation and presentation remain open. No
commercial game is supported by this checkpoint.

Public interface references: [SVC](https://switchbrew.org/wiki/SVC),
[libnx register wrappers](https://github.com/switchbrew/libnx/blob/master/nx/source/kernel/svc.s)
and [thread-local region](https://switchbrew.org/wiki/Thread_Local_Region).
Cancellation ordering was also checked against the public
[Mesosphere OS wait interface](https://github.com/Atmosphere-NX/Atmosphere/blob/master/libraries/libmesosphere/source/kern_k_synchronization_object.cpp);
its implementation is not a dependency and is not copied.
