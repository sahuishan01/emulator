# Visible original homebrew

Version0.19 introduced **Play homebrew demo** to the Android app. Catch is our
own static NRO, interpreted continuously: drag or use Left/Right to move the red
paddle under a falling green square. The guest updates positions, checks catches
and misses, tracks score, generates vertices and submits actual B197 commands.
Pause freezes guest advancement; Reset resets guest game state. Keyboard/D-pad
left/right works too. No commercial program or existing emulator core is used.

## Live execution and publication

`catch.S` initializes SM/nvdrv/transfer memory/NVMap/AS/channel once. Its imported
20KiB backing contains a4KiB command/shader/vertex page and a16KiB framebuffer.
The126-word batch clears then draws the two rectangles at64x64. Exact coordinate
lookup data avoids requiring guest FP for this sample. Original shifted-register
ADD/ADDS/SUB/SUBS (LSL/LSR/ASR, W32/X64) now cover its coordinate addressing and
collision arithmetic; ten LLVM instruction vectors and reserved-encoding tests
check zero-register, width, flags and shift behavior independently.

The live session installs a `GraphicsBackend` through SM's NV driver factory.
`PreparedSubmission::graphics_frame` preserves command order and derives an
independent serial shader/coverage image before any effects. The current host
profile explicitly requires one full clear followed by1..3 draws, no DMA, one
pitch-linear RGBA/BGRA surface <=64x64, full component mask and <=6 vertices/draw.
Unsupported combinations return errors rather than silently reorder operations.
The demo value ABI narrows this to64x64/two draws.

`PreparedServiceEffect::complete` runs **after** kernel reply encoding, handle
staging and output/TLS alias preflight. It calls Vulkan synchronously, renders
with the guest's translated SPIR-V/vertices/state, waits a real fence, invalidates
noncoherent readback where needed, and compares every byte with the serial
oracle. Only successful completion permits the already validated reference row
effects, reply, successor channel state and syncpoint to publish. These guest
writes are CPU publication of GPU-verified reference bytes; this is not general
GPU-authoritative memory/coherence or asynchronous execution. Rendering failure
leaves guest bytes/state/completion unchanged. A failed reply never calls Vulkan.
All inputs and pins remain owned across the callback/fence; callbacks cannot
reenter the thread-confined guest API.

Unlike the older replay tests, this callback runs inside each actual guest NV
submission before its CPU continues. The Android callback presents the freshly
rendered image to the window. No host code calculates ball movement or collision.

## Window and input

The renderer uses an Android Vulkan surface and FIFO swapchain, a compatible
present queue, capability-checked transfer usage/UNORM format/blit support,
nearest blits with square letterboxing, acquire synchronization and one present
semaphore per swapchain image. Frames are serialized with readback and queue
idle, deliberately favoring a simple reference lifetime model. Frame resources
and command buffers are reclaimed before the next frame. Pipeline/module/buffer
creation each frame and readback limit performance; no console speed claim.

One Java render worker owns Vulkan and the thread-local Rust session. Pause
skips execution. Backgrounding cancels frame generation and closes the guest and
renderer; foregrounding starts a fresh session. A destroyed TextureView returns
false so its texture is retained until queued Vulkan teardown completes, then
released on that worker. Generation checks discard stale scheduled frame tasks.
Swapchain/surface errors stop visibly; window recreation starts a new renderer.
The guest closes devices, channel, sessions and its transfer arena on normal
shutdown; tests assert zero live handles, transfer objects and lent bytes.

Input/frame handoff is an explicitly emulator-owned mailbox at image offset
`0xa000`: ready,input-left,reset,stop,frame,score,misses,ball-x,ball-y,paddle-left
(ten u32s). Input-left is0..48; the guest bounds it too. It sets ready only after
completed NV submission, allowing the host to pace the interpreter. This is not
Horizon HID, VI or Binder BufferQueue. The window displays the submitted demo
target by host policy; general guest-selected display queues remain open.

## ABI and evidence

The three new exports are start(callback), step(input,reset), stop, all `_v1`.
They must run on the same render thread; the trusted callback must be nonnull,
return a status, retain no borrowed state and never throw across the C boundary.
All parameters/results are owned C values: LiveDraw66068bytes/alignment4,
background offset16/draws76/pixels49684 (16384bytes); DemoStatus296bytes/alignment4,
error offset40. Rust/C++ layout assertions and both-ABI APK symbol checks cover
all ten exports. No Rust raw pointers, unsafe blocks or external Cargo crates
were introduced. A RefCell guard rejects callback reentry; export panics become
bounded errors. Normal stop is idempotent. Explicit IPC trace draining at frame
boundaries prevents the diagnostic4096-record cap ending a long-running demo.

`last-demo-report.json` records actual frame/score/input/presentation counters,
device, error, allocation count and version, using a temporary-file rename.
The normal Export report includes that capture when it matches the app version.
Its capture time matters: it records the latest observed run, not current liveness.
The standard batch adds a111-frame live Vulkan regression, for103 total checks.
It has no Android window; visible display/input/lifecycle are separately verified
by `tools/qa_homebrew.py` with fresh XML-derived input, screenshots and reports.

Remaining work includes general display services/buffer queues and HID, broader
GPU/shader/texture functionality, CPU/runtime/content loading, audio and substantial
performance work. This small playable homebrew does not run Pokémon Z-A.

References: [Arm instruction guide](https://documentation-service.arm.com/static/6245c734b059dc5ff9a8bdab),
[Khronos acquire](https://docs.vulkan.org/refpages/latest/refpages/source/vkAcquireNextImageKHR.html),
[Khronos present](https://docs.vulkan.org/refpages/latest/refpages/source/vkQueuePresentKHR.html),
[Android TextureView lifetime](https://developer.android.com/reference/android/view/TextureView.SurfaceTextureListener).

## Held controls (0.19.1)

Left/Right now move one4-unit step on touch down, then repeat after240ms every
80ms while held. Normal taps and accessibility clicks still move exactly once.
The repeat loop is owned by the UI thread, separate from rendering; guest input
still uses the same bounded mailbox. Both held directions neutralize each other.
Release, cancellation, the active pointer lifting or sliding off the button
stops that hold. Dragging the game area, Pause, loss of window focus, background,
surface recreation and destruction clear held state and pending callbacks; an
old hold never rearms on resume. Keyboard/D-pad behavior stays unchanged.
Schema4/workload20 invalidates prior cached diagnostics; native GPU workload7
and the103 cases are unchanged.
