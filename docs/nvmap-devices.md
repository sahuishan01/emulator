# Original NV device and NVMap backing subset

Version 0.12.0-dev extends our nvdrv Initialize resource subset with device
file descriptors and three metadata ioctls. All Rust, tests and guest assembly
are original. The public [NV service interface](https://switchbrew.org/wiki/NV_services)
and [libnx client declarations](https://github.com/switchbrew/libnx/blob/master/nx/source/services/nv.c)
supply wire-format facts only. Version 0.13.0-dev adds NVMAP_ALLOC imports of
existing guest backing; fresh memory allocation is still unsupported.

## Device RPCs

An initialized client accepts Open (command 0), Ioctl (1), Close (2), and the
previous Initialize (3). Open takes one normal HIPC A descriptor containing
`/dev/nvmap`, without a trailing NUL, and returns u32 fd/error. Paths are bounded
to 64 ASCII bytes. Other paths return NvError NotSupported (2); invalid paths
return BadParameter (4). Close takes a u32 fd and returns a u32 error. Calls
before Initialize return NotInitialized (3). CMIF transport success and the
NV error word are distinct. Initialize retains its earlier partial contract;
full modern libnx bootstrap, Aruid setup and CloneCurrentObjectEx remain open.

Each client permits sixteen live descriptors. IDs increase without reuse, so
closed descriptors cannot reach later objects. CMIF session clones share the
same client, including descriptors and arena ownership. Closing a descriptor
releases its owned metadata/imports; GPU mappings may retain backing after that.
Closing the final receiver releases the
client's arena and backing references. Closing the sender's transfer handle alone does not restore rights.

Ioctl takes u32 fd/request; NVMap commands use one A input and one B output.
The supported subset
requires each buffer to have exactly the ioctl argument size. Input and output
may refer to the same CPU bytes; input is captured before any output write.
Transport preflight checks all mappings/rights and rejects B-to-TLS aliases.

| Request | Arguments | Behavior |
| --- | --- | --- |
| `0xC0080101` NVMAP_CREATE | 8 bytes: size, handle output | Record an unallocated object's size and return a new handle |
| `0xC0200104` NVMAP_ALLOC | 32 bytes: handle, heapmask, flags, alignment, kind u8/padding, address u64 | Import existing guest backing with READ or RW rights |
| `0xC00C0109` NVMAP_PARAM | 12 bytes: handle, parameter, result output | Parameter 1 returns size; 2/5 return allocated alignment/kind |
| `0xC0180105` NVMAP_FREE | 24 bytes: handle, zero padding, address output u64, size output, flags output | Remove an object; return imported address (zero if unallocated or still GPU-mapped), size and zero flags |

Objects retain their owning descriptor, capped at 64 live objects per fd.
Since 0.14, handles increase in one non-reused client-wide namespace so GPU
address-space devices can resolve them without ambiguity. Wrong-fd NVMap
operations fail; same-client GPU mappings may import allocated handles. This implementation accepts positive,
4 KiB-aligned sizes up to 64 MiB as an explicit reference limit. No backing
memory is allocated or imported by Create. Unsupported parameters, ioctls and
devices return error 2; invalid fds, object handles or argument sizes return 4;
quota exhaustion returns 6. Repeated Alloc and allocated-only parameters on
unallocated objects return InvalidState (8). Failed ioctls preserve B bytes and live object state.
Malformed HIPC/CMIF and invalid descriptor rights remain typed host faults.
An invalid Alloc source address or insufficient source rights returns NV error 4.

Service preparation operates on an isolated metadata snapshot. Successful
ioctls prepare a pinned B write. All reply encoding, handle staging and effect
checks precede output/state publication. Rejected replies cancel both: Open
cannot consume a descriptor ID, Create cannot consume a handle, Free cannot
remove an object, and B stays unchanged. Tests force oversized replies to prove
these properties, including cancellation/release of candidate imported views,
alongside stale IDs, bounds, invalid paths, missing grants,
shared-session lifetime and quota reclamation.

## Backing imports in 0.13

Alloc accepts heapmask 0, flags 0 (READ) or 1 (RW), linear kind 0, seven zero
padding bytes and an existing, nonzero guest CPU address. Other heap/flag/kind
modes and address 0 (fresh allocation) explicitly return NotSupported (2).
Alignment 0 or a power of two below 4096 normalizes to 4096; larger powers of
two up to 64 MiB are supported. The address must satisfy normalized alignment.
Non-power-of-two alignment, bad padding, unmapped/overflowing addresses and
insufficient source rights fail without allocating/importing anything.

The service obtains a bounded MemoryView under the current process's existing
rights. It retains the original shared backing IDs and bytes; no payload copy,
clear, replacement or CPU permission change occurs. CPU writes are visible to
the view. CPU unmap/protect/address reuse cannot redirect it; original resident
allocation charges remain until the final pin drops. Imported capabilities do
not acquire later CPU mapping rights. This is a reference import contract,
not a full console cache-coherence/page-state implementation.

Per client, all descriptors share a 64 MiB logical imported-byte budget and
4096 backing-span budget. Aliased imports are charged for each view; cloned
session references do not count again. Object Free, device Close and final
receiver Close drop imports. Snapshot preparation temporarily clones references
without copying payloads; rejected replies drop only candidate references and
preserve live imports. Free returns zero while an NV GPU mapping retains the
backing, otherwise the recorded CPU address even if unmapped/reused; it never unmaps or frees a caller's
current CPU mapping. Supported cached modes return zero Free flags. GPU mapping references now retain their own views; see
[the address-space subset](nv-gpu-address-space.md). Sharing IDs and uncached
flags remain unsupported.

Unit and service integration tests verify actual backing read/write visibility,
read-only capabilities, address reuse, originating resident charges, byte/span
quotas across device fds, cloned-session lifetime, invalid arguments and failed
Alloc/Free/Close reply cancellation.

## Guest evidence and remaining dependencies

Our independently LLVM-assembled nvmap.nro creates a transfer arena, initializes
nvdrv, opens `/dev/nvmap`, clones the session and closes the original, creates a
4 KiB object, queries SIZE, frees it, closes the device/final session, checks
restored CPU access, and exits. Expected trace: 719 instructions, nine IPC calls,
two copied handles, four A/three B descriptors and 98 descriptor bytes. It stores
4096 at image offset 0x3030, preserves source value 42 at 0x2000, and exits with
zero handles, transfer objects or lent bytes.

The [0.12 phone report](validation/g720-nvmap.md) passes this metadata guest
and the transfer-memory guest. The new nvmap-alloc.nro adds Alloc, normalized
ALIGNMENT, imported-address Free and CPU byte preservation/update checks. It
expects 817 instructions/10 IPC/2 copied handles/5 A/4 B/162 descriptor bytes,
4096 at image offset 0x3030 and updated source 77 at 0x4000, with zero handles,
transfer objects or lent bytes. Its [0.13 phone checkpoint](validation/g720-nvmap-backing.md) passes all twenty cases.

[GPU address-space mapping](nv-gpu-address-space.md) now retains imported views.
Fresh allocation, object sharing IDs, NV channels,
syncpoints, cache control and GPU submission through NV remain open. The
separate emu:gpu reference still tests actual original linear DMA. This backing
checkpoint does not render guest shaders or launch commercial games.
