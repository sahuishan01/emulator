# Guest color clear and Vulkan replay batch

0.16.0-dev runs 60 checks in one tap: fourteen interpreted NROs, thirty-four
kernel/NV scenarios, three GPU-core cases and nine Vulkan cases. Schema 4/workload
16 still requires all four groups. This is a development diagnostic.

## Original Maxwell subset

Register identities/fields come from NVIDIA's public
[MAXWELL_B class interface](https://github.com/NVIDIA/open-gpu-doc/blob/master/classes/3d/clb197.h).
No vendor implementation or emulator core is imported. The original decoder adds
class B197 alongside DMA B0B5. NV AllocObjCtx accepts either class; submissions
cannot bind a class different from their allocated context. Standalone core
queues can explicitly switch classes, preserving original effect order.

Supported methods (byte offsets):

| Registers | Meaning |
| --- | --- |
| 0800..0820 | Target 0 address, pitch, height, format, layout, third dimension, array pitch and layer |
| 0d80..0d8c | Four finite IEEE-754 clear values in logical RGBA order |
| 0ff4/0ff8 | Zero-origin logical surface clip width/height |
| 0d6c/0d70 | Optional clear rectangle, exclusive maximum bounds |
| 0e00/0e04/0e08 | Optional enabled scissor 0 and rectangle |
| 10f8 | Clear control: clear-rectangle bit 4 and scissor bit 8 only |
| 19d0 | Color component bits 2..5; one operation per invocation |

Initial support is deliberately narrow: pitch-linear (layout 1000), target 0,
one layer/third dimension 1, zero array pitch, RGBA8 UNORM D5 and BGRA8 UNORM CF.
RT width holds row pitch bytes; surface clip supplies logical width. Height must
match the clip. Pitch is a multiple of four, at most 16384 bytes; dimensions are
positive and at most 4096. Address is a four-byte-aligned 40-bit GPU VA. All
required state is explicit; a bind resets it. The executor validates the touched
rows, rather than importing untouched padding as a writable destination.

Finite colors clamp to [0,1], with the reference UNORM8 conversion rounding to
the nearest byte. Nonfinite colors fail explicitly. Hardware tie-breaking and
other floating formats are not established. Component masks operate in logical
RGBA order, including BGRA byte swizzling. Full writes require WRITE; masked
writes also require READ for preserved channels. Clear/scissor rectangles
intersect the surface; disjoint rectangles are valid empty operations. Reversed
bounds fail. Disabled scissor has no clipping effect.

No depth/stencil clears, MRT/array selection, tiled targets, multisampling, sRGB,
integer formats, nonzero surface origins, viewport clip, stencil control,
shaders, draw methods or MME are accepted. Unsupported states/methods fail.
This does not implement a complete console graphics context.

## Transactions and ownership

Each prepared clear owns row pins and preallocated storage. All validation
finishes before memory changes. Dropping cancels the whole batch. At commit,
masked channels read current backing so earlier clear/DMA effects are visible.
Mixed copy/clear effects retain method order, and register changes are published
only with a successful batch. The existing combined limits apply to both kinds:
64 operations, 4 MiB touched bytes/scratch, 4096 retained backing spans.

Clear write ranges participate in the same ioctl-output and active TLS reply
alias rejection as DMA. A failed oversized reply publishes neither pixels,
registers nor completion. CPU unmap/address reuse cannot redirect retained pins.
Closing bound NVMap/AS fds leaves channel references alive; final cleanup releases
them. Completion remains synchronous CPU reference completion, not host GPU time.

## Workloads and host boundary

The original LLVM-assembled `nv-color-clear.nro` uses real SM/NV IPC, maps its
backing and allocates B197 context. It rejects a partly planned three-clear batch,
then submits the valid batch: blue background, red left half and green central
rectangle. It checks sampled pixels, closes bound external fds, submits another
clear using retained state, reads completion after channel close and restores
transfer memory. Host checks compare all 128 pixels with an independent formula.
Exact trace: 5571 instructions/25 IPC/2 copied handles/18 A/11 B/603 descriptor
bytes. Image is at NRO offset 5800, completion 2 at 4040, original marker 42 at
3000; all handles/transfer objects/lent bytes return to zero.

Sixteen additional NV API scenarios cover RGBA/BGRA, masks, rectangle/scissor
intersection, float clamp, read-only backing, thirteen invalid-state variants,
late errors, failed replies, B/TLS aliases, closed owners/clones, padded pitch,
context mismatch and empty clears. They execute the actual kernel/service stack
but do not interpret A64 (`executesGuestCpu:false`). Existing cases remain.

The three new Vulkan cases consume validated operation records from Rust through
`emulator_guest_clear_replay_v1(index)`. Case 0 executes the NRO and replays its
actual command bytes. Cases 1/2 replay owned BGRA and masked variants. Decoder
output drives target format, rectangles, clear color and write mask. A fixed
host vertex/fragment shader lowers the clear to attachment writes; it is **not**
a guest shader translator. Each Vulkan readback compares all 512 bytes with the
independent CPU oracle; case 0 also matches the interpreted guest's image.

This is offscreen fixture replay, not a live asynchronous guest renderer or
display handoff. The NRO itself completes against the synchronous CPU executor.
The graphics milestone requiring guest vertex/fragment shaders, draws and
Android presentation remains open. No commercial game compatibility follows.

The C/Rust replay ABI is a 1264-byte, four-byte-aligned value: four u32 header
fields, eight 60-byte operations, 512 image bytes and 256 error bytes. No pointers
or callbacks cross this boundary. Unknown fixture/panic returns failure with
zero operations. Layout offsets are asserted on both sides. Existing EngineReport
ABI 2 and its three exports remain unchanged. Desktop CMake now links the same
owned Rust archive; build it first with `cargo build --offline -p emulator-android-ffi`.

Fixture generation/validation:

```sh
python3 tools/generate_clear_fixtures.py --check
python3 tools/assemble_a64_fixtures.py --check
python3 tools/compile_gpu_shaders.py --check
```

Gradle tracks the three binary fixtures as Rust build inputs. Raw captures,
physical reports and artifacts stay ignored. Device and Android evidence belong
under docs/validation/, separate from software Vulkan/reference checks.
