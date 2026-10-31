# Transfer memory and the original NV Initialize subset

Version 0.11.0-dev adds original transfer-memory lending, copied process/transfer
handles and nvdrv command 3. This is resource ownership for the standard
Initialize request. Version 0.12.0-dev adds [device descriptors and unallocated
NVMap metadata ioctls](nvmap-devices.md); 0.13 adds imports of existing guest
backing. Fresh allocation, Aruid setup,
CloneCurrentObjectEx, remote transfer mappings and shaders/rendering remain
explicit unsupported paths. The generic CMIF CloneCurrentObject (2)
continues to share a session identity.

Public ABI references: [CreateTransferMemory](https://switchbrew.org/wiki/SVC#CreateTransferMemory),
[HIPC declarations](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/sf/hipc.h)
and [the libnx NV client wrapper](https://github.com/switchbrew/libnx/blob/master/nx/source/services/nv.c).
Only interface facts are used; implementation, tests and guest assembly are
original. The Initialize wrapper takes an input u32 size and two copied handles,
process first and transfer memory second, and expects a CMIF result without an
output payload. The full libnx nvInitialize helper also invokes operations
outside this subset, so it is not claimed to run unchanged.

## Lending and kernel objects

SVC 0x15 receives address/size/permission in X1/X2/W3 and returns success/handle
in W0/W1. This reference requires positive 4 KiB-aligned guest ranges, at most
16 MiB each, with fully committed RW source backing. Owner permissions 0/1/3
mean NONE/READ/RW. The guest TLS arena is excluded. Up to 32 live transfer objects
are permitted; copied references and receiver retention count toward lifetime,
not additional objects. Handle-capacity preflight precedes lending. Recoverable
failures publish no loan or handle and preserve CPU registers. Invalid arguments
currently produce typed host faults, consistent with other unsupported kernel
subsets; complete Horizon error-code mapping is not implemented.

CPU regions retain original permissions and weak links to a MemoryLease. While
any lease owner remains, effective CPU permissions are restricted and query
metadata reports borrowed=true. Protect/unmap/commit and new aliases of loaned
bytes fail. Final lease drop restores original rights automatically; no callback
or mutable-memory access is needed during object destruction. Borrowing does not
copy payloads or change backing IDs/resident accounting.

Creation rejects physical aliases of selected bytes, including repeated backing
inside the selected virtual range, and outstanding views of touched backing
allocations. Existing capabilities are not revoked. Consequently a disjoint pin
or loan sharing one larger backing allocation can conservatively block another
loan. This reference has no complete Horizon memory-state/page-table model;
static owned RW NRO memory is accepted for the fixture. A paged allocator and
remote MapTransferMemory/UnmapTransferMemory remain future work. Metadata edits
are staged before publication, preserving existing loans and neighboring rights.

## Incoming copied handles

The parser accepts at most four incoming copy IDs after the optional PID field
and before A/B descriptors. Move handles, copy-bearing CMIF Control requests and
other special-header bits remain unsupported. The kernel resolves all IDs before
service preparation. Current-process pseudo handle 0xffff8001 becomes an opaque
identity for the owned reference process; valid transfer handles retain their
actual objects. Sessions, threads, events and other pseudo handles are not yet
supported as incoming copies. Sender handles remain present and no new guest
handle IDs are allocated for receiver references.

The public guest PID remains 1; identity checks use distinct host-owned Rc
identities, not an untrusted PID number. ServiceContext validates process/memory
order, caller identity and transfer owner. Default services and the diagnostic
GPU bridge require zero copied handles; only explicitly supported consumers
accept them. Successful traces/engine reports add copyHandles. Bad IDs/types,
permissions, buffers or replies do not publish receiver state or traces.

## nvdrv command 3

SM GetService with nvdrv followed by three zero bytes (eight bytes total)
creates a fresh NvDriver. Only Initialize (3) is implemented. It requires zero
buffers, two copied handles, a four-byte size matching the actual transfer
object, owner permission NONE and an uninitialized receiver. The candidate
service retains the checked arena object; no device allocations or heap format
are fabricated. Guest permission restriction persists after sender-handle close
and through session clones until the final receiver reference closes. Other
initialization steps and application/service permission masks remain open.
NV device/object support is limited to the linked NVMap subset. Request-state changes commit through the existing
reply-validation transaction; rejected replies drop candidate copied references.

## Evidence

Our nv-initialize.nro creates a 4 KiB transfer arena, discovers nvdrv, initializes
it, closes the sender memory handle, clones the receiver, closes both receiver
handles, reads the restored source value 42, closes SM and exits. It takes 317
instructions/four IPC calls/two copied handles with expected debug output and
zero remaining handles, transfer objects or lent bytes. Engine suite 6 adds this
ninth guest. No actual GPU command is submitted by this initialization fixture.

Tests cover partial-range lending and last-owner restoration, RW/READ/NONE
restrictions, aliases/pins, region-capacity rollback, protected TLS, object and
handle limits, copy wire alignment/truncation, wrong IDs/types/order/size, rejected
reply cleanup, explicit unsupported device calls and interpreted guest execution.
Next: implement the standard NV device descriptor/object/ioctl path against owned
workloads; extend handle/buffer/mapping variants when those workloads need them.
