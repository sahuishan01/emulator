# Original NV GPU address-space subset

Version 0.14.0-dev opens `/dev/nvhost-as-gpu` and implements a bounded InitializeEx,
MapBufferEx and UnmapBuffer path through our original GPU memory core. This is
memory mapping, not NV channel submission or guest rendering. Interface facts
come from the public [NV interface](https://switchbrew.org/wiki/NV_services#NVGPU_AS_IOCTL_MAP_BUFFER_EX)
and [libnx client declarations](https://github.com/switchbrew/libnx/blob/master/nx/source/nvidia/ioctl/nvhost-as-gpu.c);
implementation, tests and guest assembly are original.

## Device and buffer contract

A client still permits sixteen live fds total across supported devices. Each
address-space fd owns an independent GPU space after InitializeEx. CMIF session
clones share the NV client and its fds. Ioctl direction bits now determine the
required grants: input-only requires one A/no B; inout requires one A/one B.
Known arguments must exactly match the encoded size. Extra or missing descriptors
fail transport validation. Output writes and candidate metadata publish only
after reply validation; bad replies preserve live state and B bytes.

| Ioctl | Size/direction | Implemented behavior |
| --- | --- | --- |
| `0x40284109` InitializeEx | 40, input | Initialize default GPU memory space |
| `0xC0284106` MapBufferEx | 40, inout | Map a granted NVMap backing slice and return its GPU VA |
| `0xC0084105` UnmapBuffer | 8, inout | Remove the exact mapping base, preserving the input offset in B |

The public descriptions disagree on InitializeEx's first/third word order.
This reference explicitly accepts both published default combinations: flags
0/1 and big-page setting 0/65536, unused fd/reserved zero, and three zero range
words. Customized ranges/other flags fail. Only 4 KiB buffer mappings are
implemented; accepting a default big-page setting does not enable 64 KiB maps.
Repeated initialization returns InvalidState (8); mapping before it returns
NotInitialized (3).

MapBufferEx accepts automatic placement, flags 0 or cacheable hint 4, linear
kind 0/default -1, and page size 0/4096. It normalizes output kind/page size to
0/4096. The buffer offset and positive mapping size must be 4 KiB-aligned and
fit the imported view. Size zero selects the remaining view from the offset.
The non-fixed offset input is an alignment: zero or a power of two up to 64 MiB,
normalized to at least 4096. Fixed/reserved placement and other modes return
NotSupported (2). Invalid bounds, identities or addresses return BadParameter (4).

Automatic GPU VAs use first fit beginning at 4 GiB within the core's 40-bit range;
unmapped holes may be reused. Per address-space fd: 64 live mappings. Per NV
client across address spaces: 64 MiB logical mapped bytes/4096 backing spans.
These mapping budgets are separate from the NVMap import budget. Aliased maps
count individually. Quota failures return 6 and publish no mapping or output.
Unmap accepts only an exact live base; interior, stale and unknown bases return 4.

## Ownership and transaction model

NVMap IDs now increase in one client-wide namespace, replacing per-fd counters.
Each object retains its owning NVMap fd; wrong-fd metadata/Free requests fail.
The address-space device resolves allocated handles across the same client's
NVMap fds. Cross-client sharing/export IDs remain unsupported. Closing a NVMap
fd cannot let later objects reuse its IDs or redirect existing mappings.

GPU maps retain narrow MemoryViews of the original backing. GPU-core reads and
writes observe shared CPU bytes; read-only imports cannot gain GPU write rights.
CPU unmap/protect/address reuse cannot redirect captured views. Free removes the
NVMap handle but returns address zero while an NV mapping still owns its backing;
size is returned and cached flags stay zero. Without a retained mapping it returns
the recorded CPU address as before. Closing the NVMap fd also leaves GPU pins
alive. Exact Unmap, address-space fd Close or final NV receiver Close releases
GPU views. Originating resident charges persist until the last view drops; no
Free or Unmap changes the caller's current CPU mapping.

GPU address spaces expose an explicit speculative metadata snapshot preserving
space identity. Only one branch may be published; preparation must not write
through shared backing. Mapping IDs use a non-reused host allocator across all
snapshots. Discarded candidates can consume opaque host IDs, but cannot reuse or
retarget them. Guest VAs, fds, NVMap IDs, live maps and bytes roll back on failed
replies. Tests cover canceled initialization/map/unmap/close replies, source
rights, partial views, CPU address reuse, client-wide quotas, stale IDs, cloned
receiver lifetimes and final backing release. This remains one host executor;
complete console page-state/cache coherence and asynchronous GPU timing are open.

## Guest and device evidence

Our nv-gpu-map.nro creates/imports backing, opens/initializes an address-space fd,
maps a buffer, checks the returned VA/page size, clones the NV session, frees the
NVMap handle and closes its device while mapped, verifies that remapping the stale
handle returns error 4, unmaps, closes resources and checks CPU bytes/restored
transfer rights. Exact oracle: 3172 instructions, fifteen IPC calls, two copied
handles, nine A/six B descriptors and 372 descriptor bytes. Expected memory:
4 GiB at image offset 0x3040, source 77 at 0x4000 and restored marker 42 at 0x2000.
It exits with zero handles, transfer objects or lent bytes. The copy loop is
independently assembled with LLVM; existing fixtures/oracles are unchanged.

The [0.13 phone report](validation/g720-nvmap-backing.md) passes all twenty prior
ARM64/G720 cases. This new mapping guest awaits phone evidence. Channel binding,
VA reservation/fixed maps, large pages, sharing IDs, remap/cache/fence interfaces,
NV GPFIFO submission, guest shaders/rendering and commercial game support remain
open. The separate emu:gpu diagnostics still execute our original synchronous DMA.
