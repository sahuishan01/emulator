# Original GPU command framing

`crates/gpu` starts the guest GPU path with an original bounded parser for
captured Maxwell channel packets. GPU mappings use our memory crate, with no
kernel, Vulkan or external crate dependency. It decodes method writes; execution,
completion signals, driver access and rendering remain separate work.

Interface fields were checked against NVIDIA's public
[MAXWELL_CHANNEL_GPFIFO_A declarations](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/host/clb06f.h).
No existing emulator core or implementation is copied, linked or imported.
The reference is a hardware interface, not the implementation baseline.

## Supported input

An eight-byte little-endian GPFIFO entry contains a 40-bit GPU virtual address
and a 21-bit length in words. This subset accepts unconditional user/main-level
fetches without sync-wait. Conditional, privileged, subroutine, reserved-bit,
zero-length control and sync-wait entries fail explicitly. A fetch is limited
to 16384 words (64 KiB), and an address range must remain within 40 bits.
These size limits are prototype policy, not hardware limits.

GPU virtual addresses are distinct `GpuVa` values, not host pointers. Parsing
does not resolve or map them. `GpFifoEntry::decode` requires exactly the declared
capture length; future GPU address-space code must separately validate/read the
backing memory and retain its identity. The new
[GPU memory/submission layer](gpu-memory-submission.md) provides owned mappings
and immutable command capture through separate internal APIs.

The pushbuffer parser supports eight subchannels and four modern packet forms:

| Form | Decoded method writes |
| --- | --- |
| Incrementing | Each payload word advances the method word index |
| Non-incrementing | Every payload targets the same method |
| Increment once | First payload targets M; subsequent payloads target M+1 |
| Immediate | One write with the 13-bit inline value and no following payload |

The exact zero word is recognized as the documented no-op. Other legacy,
control, reserved and end-segment forms fail explicitly. Method word indices
are 12 bits; byte offsets are four times those indices. Increment overflow is
rejected rather than wrapped. Partial words, truncated payloads and reserved
packet bits are rejected. A write budget caps output at at most 16384 writes.
All work is bounded by input/output limits, and errors expose no partial output.

The standalone parser requires complete captured packets. The decode queue
joins entries within a batch. Continuation across batches, ring control,
conditional fetching and synchronization remain unimplemented.
The separate [DMA executor](gpu-dma-methods.md) supports direct class binding
and a linear-copy method subset. An accepted packet
does not imply its method has supported GPU semantics.

## Evidence and next work

Tests use original literal wire vectors independently of the decoder. They
cover all four forms, subchannel/method/payload ordering, 40-bit addresses,
declared capture length, every truncated prefix of a packet, unsupported entry
flags/opcodes, address and method overflow, and input/output budgets. The GPU
parser is tested on the host and through the separate Android `gpuCore` group.

Bounded GPU mappings and decoded submission ownership are implemented. Next:
needed NV service/ioctl/object semantics and broader class methods. Original
linear DMA copies execute through per-subchannel bindings. Shader decoding/IR/
SPIR-V, host renderer integration and presentation follow. M4's full
guest-to-image checkpoint remains open; the six Vulkan lab workloads continue
to use host shaders.
