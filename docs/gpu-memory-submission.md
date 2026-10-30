# Shared backing and decoded submission ownership

Version 0.7.0-dev adds original CPU/GPU shared backing, GPU virtual mappings and
a bounded queue of captured/decoded commands. These internal APIs do not execute
GPU methods, shaders or fences and are not a completed NV service/ioctl path.

## Storage and access

CPU regions retain slices of fixed-length owned backing with opaque stable IDs.
Imported Vec spare capacity is dropped before backing is retained. Protect/unmap
split metadata without copying bytes. `AddressSpace::pin` checks the complete
committed range and requested rights, returning a capability-limited MemoryView.
CPU aliases and GPU mappings observe the same bytes. Existing views retain
their granted rights and original backing after CPU protection/unmap or address
reuse; new views must pass the current CPU permissions.

`mapped_bytes` counts logical CPU mappings including aliases. `resident_bytes`
counts originating allocations until their last view disappears, including GPU
mappings and queue pins. Both have a configured cap, preventing unmap/remap
from evading the origin's resident budget. A surviving fragment retains its
whole original allocation; a paged allocator remains future work. These counts
exclude other host/driver heap allocations. Rc/RefCell supports one host executor;
no public backing borrow or raw pointer escapes. Concurrent renderer ownership
and host Vulkan coherence require additional synchronization.

GPU addresses are separate 40-bit GpuVa values. Mapping IDs are never reused.
Empty/overflowing/overlapping ranges, byte-budget excess, more than 1024 mappings
or 4096 backing spans fail. Reads/writes preflight every mapping before touching
bytes. Execute access is unsupported. Unmapping requires the exact mapping ID;
old pinned ranges cannot be redirected by reusing a GPU virtual address.

## Queue ownership and limits

A queue is bound to one GPU address space. Admission validates GPFIFO entries,
pins readable ranges, captures immutable command bytes and decodes the joined
stream. A packet may continue across entries within the same batch. CPU writes
after admission affect live storage but not captured commands. This is an
explicit reference capture policy, not asynchronous hardware fetch timing.

An invalid late entry/range/packet or exhausted budget publishes no partial
batch or ID. Submission IDs include queue identity; cross-queue and out-of-order
release fail. Original backing remains retained until FIFO `release_front`,
even after CPU/GPU unmap and address reuse. Releasing decoded work does not
signal GPU completion, advance a guest syncpoint or execute a method.

Hard ceilings: 256 entries and 64 KiB captured bytes per batch; 16 pending batches,
1 MiB captured bytes, 16384 decoded writes and 4096 retained spans per queue.
Smaller host budgets work. Continuation across separate batches is unsupported.

## Evidence and remaining work

Tests cover identity/alias visibility, protection and unmap, address reuse,
permissions/bounds, atomic failures, pinned resident limits, entry-spanning
packets, immutable capture, ownership, queue limits and ordered release. Existing
CPU/loader/kernel tests also run against shared backing.

Android adds a separate `gpuCore` group with two actual memory/decode cases.
The alias case releases all backing. The queue case captures two batches/four
writes, retains the first across address reuse and ends with zero resident
bytes/pending work. These are distinct from six guest NROs and six host Vulkan
cases; they do not establish guest graphics.

Next: NV service/ioctl/object registration, class routing/method execution,
synchronization, original shader decode/IR/SPIR-V and renderer integration.
M4's guest-program-to-image gate remains open.
