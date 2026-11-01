# NV channel, submission and completion batch

0.15.0-dev combines channel setup/binding, DMA object context, FIFO configuration,
GPFIFO submission, persistent method state, completion polling, rollback and
resource lifetime into one APK. Select **Run all checks** once: thirteen A64
programs, eighteen NV protocol scenarios, three GPU-core cases and six Vulkan
cases run together. The screen summarizes 40 outcomes and Export saves one JSON.
Schema 4/workload 15 adds `nv`; combined allPassed requires all four groups.
This follows the user's request to avoid a phone cycle for every small feature.

## Implemented protocol

Public interface facts come from [libnx channel declarations](https://github.com/switchbrew/libnx/blob/master/nx/source/nvidia/ioctl/nvchannel.c)
and [NV interface research](https://switchbrew.org/wiki/NV_services#NVGPU_IOCTL_CHANNEL_SUBMIT_GPFIFO).
All implementation, harnesses and assembly are original.

| Operation | Ioctl | Reference behavior |
| --- | --- | --- |
| SetNvmapFd | `0x40044801` | Bind a channel to a live same-client NVMap allocator once |
| BindChannel on AS | `0x40044101` | Bind a channel once to an initialized address space |
| AllocObjCtx | `0xC0104809` | One actual DMA context (class B0B5, flags zero); other engines fail |
| AllocGpfifoEx2 | `0xC020481A` | Configure bounded FIFO capacity and return the channel completion point |
| SubmitGpfifo | `0xC0nn4808` | Decode inline entries, preflight and execute original DMA methods |
| SyncptRead on ctrl | `0xC0080014` | Read the actual completed value |
| SyncptWaitEx | `0xC0100019` | Poll an already completed threshold; future thresholds return timeout |

Open now supports `/dev/nvhost-gpu` and `/dev/nvhost-ctrl`. Fd limit remains 16
per client. FIFO capacity is a positive power of two up to 256 entries. The
supported Ex2 client shape has flags zero, the third input word 0/1 and other
reserved words zero. This is logical capacity for the synchronous reference;
there is no asynchronous hardware ring or host Vulkan submission.

Inline Submit's encoded argument size must match 24 + 8*num_entries and both
A/B descriptors. Entry count must fit capacity; the pointer field is ignored.
Flags 2 (fence_get) and 3 (fence_get + already-satisfied fence_wait) are supported.
Other modes fail explicitly. Each accepted job advances its client-local point
once after actual DMA effects; this is a reference completion policy, not console
hardware counter timing. Submitted methods and registers persist per channel.
Rejected jobs cannot publish class/register state, data, output or completion.

Completion IDs increase without reuse, capped at 32 lifetime channels per client.
Closed-channel point values remain readable until the final NV client closes.
WaitEx supports timeout zero only: completed thresholds return the current value;
unmet ones return NV timeout 5, leaving B unchanged. Positive timeouts and other
asynchronous wait/event/cache/fence modes remain unsupported. Counter overflow
fails rather than wrapping; complete console syncpoint semantics remain open.

Channels retain allocator and address-space references. Closing their external
fds leaves referenced metadata/maps alive and charged against existing budgets.
The final channel close collects detached owners/spaces and releases backing;
open external fds can continue owning them. CPU unmap/address reuse cannot
redirect mapped commands or data. Session clones share channel state and points.
No writable shared metadata is introduced into service snapshots.

GPU preflight returns an isolated successor method state and a cancelable effect.
The combined effect commits GPU copies before ioctl output; kernel publication
then makes channel/fence state visible. DMA destinations may not overlap B output
or the active TLS reply backing. Invalid sizes, rights, commands, waits and
oversized replies leave live state/bytes unchanged. Unsupported class 3D/compute
contexts are rejected, not fabricated.

## One-run coverage

The interpreted nv-channel-batch.nro performs setup, rejects a partially planned
bad job and a launch without published state, executes valid/repeated jobs,
checks completion, rejects a future wait, polls a completed fence, closes bound
fds, submits again and reads completion after channel destruction. Exact oracle:
7179 instructions/32 IPC/2 copied handles/25 A/18 B/875 descriptor bytes; completion
3 at image offset 0x4040, source/target 44 at 0x5100/0x5108, original marker 42 at
0x3000, known output and zero handles/transfer objects/lent bytes.

The separate `nv` suite runs eighteen fresh kernel/NV scenarios on the phone:
setup/rebinding; real DMA/fences; cloned persistent state; whole-job rejection;
readonly destinations; malformed entries/capacity; unsupported contexts; fence
waits; failed-reply rollback; B/DMA and TLS/DMA aliases; detached AS submission;
CPU address reuse; final backing release; channel isolation; lifetime quotas;
prerequisites; and failed setup/open rollback. These are actual kernel API cases,
not interpreted A64 programs (`executesGuestCpu:false`). They report observed IPC
counts, verified bytes and post-exit handle counts; failures retain error text
and unavailable cleanup counts are null. A failed case does not hide later cases.

`tools/qa_android_batch.py` automates upgrade invalidation, a single full run,
reference comparisons, summary verification, fresh-XML taps, exact document export,
force-stop/cold restore, screenshots and logs. It only targets an explicitly owned
AVD, uses bounded retries/timeouts, and optionally shuts that AVD down. Raw files
remain ignored. UI summaries are derived from actual native reports, not fixed
success totals. Missing groups show incomplete checks.

ABI 2 remains an 8200-byte value with an 8192-byte payload. A third versioned
export `emulator_run_nv_checks_v2` supplies the new report. Rust panics stay within
the reporting boundary. No raw pointers or unsafe core code are added.

The [0.14 phone report](validation/g720-nv-gpu-map.md) passes all 21 prior cases.
This larger batch awaits its phone report. Fixed/large-page maps, full NV bootstrap,
async scheduling/events, broader CPU ISA, guest shaders/rendering, audio/input,
content support and commercial games remain unfinished. The six Vulkan cases
still use native host shaders; they do not render these guest commands.

## Later extension

[0.16 color-clear batch](color-clear-batch.md) also accepts a B197 object context
and rejects class binds differing from the channel's allocated context. Its
render-target/clear subset shares the same preflight/rollback/ownership and
synchronous completion rules. The earlier counts above describe 0.15.
