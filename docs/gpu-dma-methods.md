# Original Maxwell DMA method execution

Version 0.8.0-dev adds a synchronous, CPU-executed reference implementation of
one Maxwell copy-engine subset. It runs decoded GPU commands against shared
CPU/GPU backing. It does not submit host Vulkan commands or execute shaders.
NV service/ioctl/object registration and guest-program submission remain open.

## Interface and supported subset

Register offsets and launch fields follow NVIDIA's public
[MAXWELL_DMA_COPY_A header](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/dma-copy/clb0b5.h).
The implementation and tests are original Rust; no existing emulator core or
vendor implementation is used. Command framing follows the separate
[MAXWELL_CHANNEL_GPFIFO_A interface](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/host/clb06f.h).

Each queue retains eight independent subchannel bindings/register sets across
batches. Method byte offset 0 binds class `0xB0B5` directly; rebinding resets that
subchannel. Class NOP at `0x100` has no effect. Object handles and engine-selection
bits are rejected until an NV object registry exists. Supported DMA registers:

| Byte offset | Meaning |
| --- | --- |
| `0x400`, `0x404` | Source upper 8 bits and lower 32 bits |
| `0x408`, `0x40c` | Destination upper 8 bits and lower 32 bits |
| `0x418` | Byte length |
| `0x300` | Launch |

All five address/length registers must be initialized. Launch accepts exactly
`0x182`: non-pipelined, one-line, pitch-to-pitch transfer using virtual addresses.
The length must be positive. Pitches/line count, multiline/block-linear/remap,
physical addressing, pipelined transfer, semaphores, interrupts, flush/cache
controls and other methods/classes fail explicitly. Overlapping source and
destination backing ranges fail, even through distinct virtual aliases; overlap
semantics are not guessed. Adjacent non-overlapping ranges are supported.

## Transaction and ownership

`SubmissionQueue::execute_front(space, id, limits)` requires the owning address
space and exact front submission ID. It clones method state, validates the whole
batch, resolves/pins every data range with its required permissions and allocates
all scratch buffers before touching memory. A late error changes neither data,
method state nor the queue. The failed front remains available for explicit
`release_front` discard or retry. Discard does not execute anything.

A successful batch executes copies in order and consumes its front exactly once.
Later copies read earlier writes, including via another virtual alias. Command
bytes were captured at admission; data mappings and bytes resolve at execution.
The single-thread reference executor has no callbacks or escaped backing borrows.
Prepared pins retain immutable rights and lengths, making exact-sized commit
accesses infallible under those invariants. Internal invariant violations panic;
this is not crash isolation or a concurrent renderer implementation.

Hard per-batch execution ceilings are 64 launches, 4 MiB copied/scratch bytes and
4096 retained source/destination backing spans. Smaller positive host limits are
accepted. Queue capture limits apply separately. Completion statistics report
method writes, copy launches and copied bytes only; no guest syncpoint, fence,
interrupt or native GPU completion is signaled.

## Evidence and next dependency

Eight new Rust tests cover execution-time data, command capture, independent
subchannels, rebind/reset, dependent alias copies, late-error rollback, supported
bounds, unsupported fields, permissions, overlaps and FIFO/address-space ownership.
Android `gpuCore` suite 2 adds `maxwell-dma-linear-copy`: one captured batch,
11 decoded writes, two dependent launches, 16 copied bytes and zero remaining
resident backing/pending work. Its byte oracle uses CPU reads, not a second copy
implementation. Six guest NROs and six host Vulkan checks remain separate.

Next work is the bounded NV service/ioctl/object path needed for an original
homebrew program to submit these commands. Original shader decode/IR/SPIR-V and
renderer integration follow; the guest-program-to-image milestone remains open.
