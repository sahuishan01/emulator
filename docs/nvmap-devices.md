# Original NV device and unallocated NVMap subset

Version 0.12.0-dev extends our nvdrv Initialize resource subset with device
file descriptors and three metadata ioctls. All Rust, tests and guest assembly
are original. The public [NV service interface](https://switchbrew.org/wiki/NV_services)
and [libnx client declarations](https://github.com/switchbrew/libnx/blob/master/nx/source/services/nv.c)
supply wire-format facts only.

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
releases its metadata; closing the final receiver releases the client's arena
reference. Closing the sender's transfer handle alone does not restore rights.

Ioctl takes u32 fd/request, one A input and one B output. The supported subset
requires each buffer to have exactly the ioctl argument size. Input and output
may refer to the same CPU bytes; input is captured before any output write.
Transport preflight checks all mappings/rights and rejects B-to-TLS aliases.

| Request | Arguments | Behavior |
| --- | --- | --- |
| `0xC0080101` NVMAP_CREATE | 8 bytes: size, handle output | Record an unallocated object's size and return a new handle |
| `0xC00C0109` NVMAP_PARAM | 12 bytes: handle, parameter, result output | Parameter 1 returns the recorded size |
| `0xC0180105` NVMAP_FREE | 24 bytes: handle, zero padding, address output u64, size output, flags output | Remove an unallocated object; address/flags are zero and size is returned |

Objects are local to a descriptor, capped at 64 live objects. Handles increase
without reuse within that descriptor. This implementation accepts positive,
4 KiB-aligned sizes up to 64 MiB as an explicit reference limit. No backing
memory is allocated or imported by Create. Unsupported parameters, ioctls and
devices return error 2; invalid fds, object handles or argument sizes return 4;
quota exhaustion returns 6. Failed ioctls preserve B bytes and live object state.
Malformed HIPC/CMIF and invalid memory rights remain typed host faults.

Service preparation operates on an isolated metadata snapshot. Successful
ioctls prepare a pinned B write. All reply encoding, handle staging and effect
checks precede output/state publication. Rejected replies cancel both: Open
cannot consume a descriptor ID, Create cannot consume a handle, Free cannot
remove an object, and B stays unchanged. Tests force oversized replies to prove
these properties, alongside stale IDs, bounds, invalid paths, missing grants,
shared-session lifetime and quota reclamation.

## Guest evidence and remaining dependencies

Our independently LLVM-assembled nvmap.nro creates a transfer arena, initializes
nvdrv, opens `/dev/nvmap`, clones the session and closes the original, creates a
4 KiB object, queries SIZE, frees it, closes the device/final session, checks
restored CPU access, and exits. Expected trace: 719 instructions, nine IPC calls,
two copied handles, four A/three B descriptors and 98 descriptor bytes. It stores
4096 at image offset 0x3030, preserves source value 42 at 0x2000, and exits with
zero handles, transfer objects or lent bytes.

NVMAP_ALLOC, backing import, object sharing IDs, pinning, GPU address-space
mapping, NV channels, syncpoints and GPU submission through NV remain open.
The separate emu:gpu reference still tests actual original linear DMA. This
metadata checkpoint does not render guest shaders or launch commercial games.
