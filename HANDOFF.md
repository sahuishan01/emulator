# Handoff

## Working Agreement & Invariants

- **Original Clean-Room Core**: Emulator written completely from scratch. Do not fork, link, copy, import, or vendor code from `upstream/` or third-party emulator cores. `upstream/` is purely historical reference and excluded from builds.
- **Target Hardware**: Realme GT 7 Dream Edition (`RMX5061` / `MT6989` / `Mali-G720-Immortalis MC12`). Eventual target game: Pokémon Legends: Z-A. Commercial games do not boot yet.
- **Execution Model**: Autonomous progress. Once a milestone or checkpoint is verified, proceed directly to the next planned task without repeatedly asking for user go-aheads.
- **Batch Verification**: Batch connected features, positive tests, rejections, rollbacks, and lifecycle coverage into single APK builds.
- **System Constraints**:
  - Maintain strict 16 KiB page-alignment across `arm64-v8a` and `x86_64` native libraries in APK builds.
  - Export all 22 required native engine symbols.
  - Respect CPU-gating (run heavy operations only when CPU idle $\ge 80\%$).
- **Notifications**:
  - `agent-releases`: Sent ONLY after successful build verification and publishing, containing permanent release-page and download links.
  - `agent-tasks`: Sent at the conclusion of tasks summarizing actions taken.

---

## Active Status & Latest Releases

- **Current Git HEAD**: `HEAD` on `main` (tracked at origin `sahuishan01/emulator`).
- **Latest Release**: `v0.28.36-dev` (versionCode 76).
  - **Live Kernel SVC & IPC Telemetry Observability**:
    - Instrumented `Kernel` (`crates/kernel/src/lib.rs`) with live metric tracking: `svc_count`, `last_svc_id`, `last_svc_pc`, `ipc_count`, and `last_ipc_cmd`.
    - Exposed telemetry accessors in `Kernel` and threaded metrics through `crates/android-ffi/src/program.rs` in `emulator_program_step_v1` JSON reports (`svcCount`, `lastSvc`, `lastSvcPc`, `ipcCount`, `lastIpcCmd`).
    - Extended `ProgramActivity.kt` execution telemetry: periodic logging now records live supervisor calls and IPC commands (`svcs=$svcCount(last=0x$lastSvc@0x$lastSvcPc), ipcs=$ipcCount(lastCmd=$lastIpcCmd)`), enabling instant diagnosis of whether guest runtime linker (`rtld`) is actively querying system state or stalled in user-space logic.
  - **Guest Diagnostic Break & Output Logging**:
    - Updated `SVC_BREAK` (0x26/0x2C) in `crates/kernel/src/lib.rs` to record the guest Program Counter (PC) where the break occurred (`guest svcBreak: reason=..., info1=..., info2=... at pc=0x...`).
    - Added dedicated logging in `ProgramActivity.kt` (`AppLog.i("ProgramActivity", "Guest diagnostic output: $guestOutput")`) so any guest assertion breaks, debug strings, or panic outputs appear immediately in diagnostic logs and logcat.
  - **MIPS Rate Tracking & Telemetry Fix**:
    - Fixed MIPS calculation in `ProgramActivity.kt`: preserved `currentMips` across sample iterations rather than losing the calculated rate when sample intervals are under 400ms, ensuring real-time MIPS is accurately displayed on HUD and reported in logs.
  - **Invariants & Verification**:
    - Maintained strict 16 KiB page-alignment across `arm64-v8a` and `x86_64` native libraries in APK builds.
    - Exported and verified all 22 required engine symbols.
    - 100% pass rate across entire Rust workspace test suite and Android unit/verification tests.
- **Previous Release**: `v0.28.35-dev` (versionCode 75).
  - **Horizon OS Kernel Supervisor Call (SVC) ABI Corrections**:
    - `SVC_QUERY_MEMORY` (0x06): Fixed page-specific information output pointer handling. When callers provide an `out_page_info` pointer in `cpu.x[1]`, the kernel now writes `0_u32` through the pointer to guest memory (`memory.write_u32(in_pageinfo_addr, 0)`) in addition to returning `cpu.x[1] = 0` and `cpu.x[0] = 0`, eliminating uninitialized stack reads during runtime link-editor (`rtld`) memory scans.
    - `SVC_GET_INFO` (0x29): Corrected register return ABI to return queried values in `cpu.x[1]` alongside returning `cpu.x[0] = 0` (and dual-writing to `out_ptr` in `cpu.x[0]` when aligned). Prevents userspace wrappers from clobbering queried results with the input `info_type`.
    - Implemented full coverage for extended InfoTypes: `SystemResourceSizeTotal` (16), `SystemResourceSizeUsed` (17), `ProgramId` (18), `InitialProcessIdRange` (19), `UserExceptionContextAddress` (20), `TotalNonSystemMemorySize` (21), `UsedNonSystemMemorySize` (22), `IsApplication` (23 returning 1), `FreeThreadCount` (24 returning 128), `ThreadTickCount` (25), and `IsSvcPermitted` (26 returning 1).
    - `SVC_GET_PROCESS_ID` (0x24) & `SVC_GET_THREAD_ID` (0x25): Aligned register return ABI to deliver IDs in `cpu.x[1]` with `cpu.x[0] = 0` (with dual-write to `out_ptr` when valid).
    - `SVC_SET_HEAP_SIZE` (0x01): Aligned to deliver heap base in `cpu.x[1]` and support optional pointer write in `cpu.x[0]`.
  - **Address Space & ASLR Boundary Synchronization**:
    - Synchronized `InfoType_AslrRegionAddress` (12) and `InfoType_AslrRegionSize` (13) with `LOAD_BASE` (`0x10_0000`) and the active process layout, resolving address-range mismatches where `rtld`'s module discovery scan previously skipped loaded NSOs outside the reported ASLR space.
    - Connected `InfoType_HeapRegionAddress` (4) to dynamically report `kernel.heap_base`.
    - Added `kernel.set_aslr_region()` and connected NPDM `title_id` to `InfoType_ProgramId` in `Session::from_exefs_with_services`.
  - **Invariants & Verification**:
    - Maintained strict 16 KiB page-alignment across `arm64-v8a` and `x86_64` native libraries in APK builds.
    - Exported and verified all 22 required engine symbols.
    - 100% pass rate across entire Rust workspace test suite and Android unit/verification tests.
- **Previous Release**: `v0.28.34-dev` (versionCode 74).
  - **NVIDIA Tegra X1 Maxwell Architecture Discovery & Characteristics**:
    - Completed full 160-byte `GET_CHARACTERISTICS` ioctl (`0x80044705`) on `/dev/nvhost-ctrl-gpu`: reports GM20B architecture (`arch: 0x120`, `impl: 0x0B`, `rev: 0xA1`), `threed_class` (`0xB197` MAXWELL_B), `compute_class` (`0xB1C0` MAXWELL_COMPUTE_B), `gpfifo_class` (`0xB06F`), `inline_to_memory_class` (`0xA140`), `dma_copy_class` (`0xB0B5` MAXWELL_DMA_COPY_A), big pages (64 KiB), and multi-processor counts. Resolves NVN runtime initialization failure when querying 3D and compute engine classes.
    - Implemented `GET_TPC_MASKS` (`0x800c4706` / `0x80044706`) returning active TPC bitmask `0x3` (both TPCs active on Tegra X1).
    - Implemented `SYNCPT_INCR` (`0xc0040015` / `0x80040015`) on `/dev/nvhost-ctrl` for direct guest syncpoint increments.
  - **Maxwell Compute Class & Flexible GPFIFO Submissions**:
    - Added `MAXWELL_COMPUTE_B` (`0xB1C0`) constant in `crates/gpu`, supporting compute engine context allocations (`ALLOC_CONTEXT`) and subchannel engine bindings (`EngineState`).
    - Relaxed GPFIFO submission flags in `nvchannel.rs`: supports flags `0` (none), `1` (wait only), `2` (get only), and `3` (wait + get), with conditional syncpoint advancement strictly when `fence_get` (bit 1) is active.
  - **Horizon Applet Proxy & System Settings Extensions**:
    - Aligned `IApplicationProxy::GetSelfController` (command 1) to always deliver `ISelfController` in `moved_objects`, while preserving `user_id = 1u64` in `data` for legacy compatibility.
    - Extended `set:sys` with `GetColorSetId` (3) returning `BasicBlack` (0) and `GetNotificationVolume` (13) returning `1.0f32`.
  - **Invariants & Verification**:
    - Maintained strict 16 KiB page-alignment across `arm64-v8a` and `x86_64` native libraries in APK builds.
    - Exported and verified all 22 required engine symbols.
    - 100% pass rate across entire Rust workspace test suite and Android unit/verification tests.
- **Previous Release**: `v0.28.33-dev` (versionCode 73).
  - **Horizon OS Applet Manager (`appletOE` / `appletAE`) Controller Architecture**:
    - Implemented full clean-room `IApplicationProxy` sub-interface hierarchy (`CommonStateGetter`, `SelfController`, `WindowController`, `AudioController`, `DisplayController`, `LibraryAppletCreator`, `ApplicationFunctions`, `DebugFunctions`), resolving startup crashes when official Nintendo SDK CRT calls `OpenApplicationProxy` (command 200) and attempts to query child controllers.
    - `ICommonStateGetter`: Implemented `GetEventHandle` (0) returning kernel readable event handle, `ReceiveMessage` (1) returning focus change message `0x0F`, `GetOperationMode` (5) returning Handheld (0), `GetPerformanceMode` (6) returning Handheld (0), and `GetCurrentFocusState` (9) returning InFocus (1).
    - `ISelfController`: Implemented `Exit` (0), `LockExit` (1), `UnlockExit` (2), `GetLibraryAppletLaunchableEvent` (9) returning kernel event handle, `CreateManagedDisplayLayer` (40) returning layer ID 1, `CreateManagedDisplaySeparableLayer` (41), screen shot and operation mode change notification handlers.
    - `IWindowController`: Implemented `GetAppletResourceUserId` (1) returning user ID 1 and `AcquireForegroundRights` (10).
    - `IAudioController`: Implemented `SetExpectedMasterVolume` (0) and `GetMainAppletExpectedMasterVolume` (1) returning 1.0f32.
    - `IApplicationFunctions`: Implemented `InitializeApplicationCopyrightFlag` (1), `EnsureSaveData` (20), `GetDesiredLanguage` (21) returning "en-US", `SetTerminateResult` (22), `GetDisplayVersion` (23) returning "1.0.0", `NotifyRunning` (40), `GetPseudoDeviceId` (50) returning 16-byte pseudo UUID, and gameplay recording stubs (66, 67).
  - **Horizon OS Event Handle IPC Abstraction**:
    - Created `EventHandle` (`crates/services/src/event.rs`) bridging kernel `Object::ReadableEvent` to guest IPC responses, enabling services (`applet`, `nvdrv`, `time`) to pass real waitable event handles back to guest threads via `ServiceReply.moved_objects`.
  - **Horizon OS System Settings (`set:sys` / `set`) Extended Primitives**:
    - Implemented `GetSystemLanguage` (2) returning AmericanEnglish (1), `GetPrimaryAlbumStorage` (5) returning SdCard (1), `GetProductModel` (8) returning Erista (1), `GetDeviceNickName` (11) returning "Switch", and `GetFirmwareVersion` (14) returning 0x100-byte Horizon OS 18.1.0 struct.
  - **Horizon OS Steady Clock & Time Services (`time:s` / `time:u` / `time:a`)**:
    - Implemented `SystemClockService` (`GetStandardUserSystemClock` 3, `GetStandardNetworkSystemClock` 4), `SteadyClockService` (`GetStandardSteadyClock` 5), `TimeZoneService` (`GetTimeZoneService` 6), and `GetSharedMemoryNativeHandle` (20).
  - **Horizon OS NVIDIA Driver (`nvdrv` / `nvdrv:a` / `nvdrv:s` / `nvdrv:t`) Extended Commands**:
    - Implemented `QueryEvent` (4) returning syncpoint event handles for guest synchronization, `SetClientPID` (8), `DumpGraphicsMemoryInfo` (11), and multi-buffer ioctl routing (`Ioctl3` 13).
  - **Filesystem Proxy (`fsp-srv` / `fsp-pr`) & Service Port Aliasing**:
    - Implemented `OpenDataStorageByProgramId` (202), `OpenDataStorageByDataId` (203), `OpenRomStorage` (204), and `SetGlobalAccessLogMode` (1005).
    - Registered official service port aliases `time:a`, `nvdrv:t`, `fsp-pr`, and `apm:sys`.
- **Previous Release**: `v0.28.32-dev` (versionCode 72).
  - **Dynamic Surface Lifecycle & Worker Thread Handover**:
    - Replaced asynchronous executor-task queueing in `ProgramActivity.kt` (`onSurfaceTextureAvailable`, `onSurfaceTextureSizeChanged`, `onSurfaceTextureDestroyed`) with lock-free atomic handover (`pendingSurface` / `surfaceNeedsUpdate`).
    - Resolved critical issue where `NativePrograms.setSurface` was starved from executing because `worker` was occupied by the continuous execution loop: surface changes are now dynamically applied by the worker thread itself at the top of each execution step.
    - Updated `onSurfaceTextureDestroyed` to return `true`, allowing Android to synchronously and safely reclaim the `SurfaceTexture` without asynchronous release tasks or uncaught `RejectedExecutionException` during Activity destruction.
  - **Live Boot Telemetry & Instruction Counter HUD**:
    - Updated `GameViewportView.onDraw`: dynamically displays the live instruction count and MIPS rate beneath `BOOTING GUEST RUNTIME…` during the initial guest boot phase before frames are presented, making execution progress immediately visible to the user.
    - Added periodic `AppLog.i` status logging every 2.5 seconds during execution (logging state, instructions, PC, MIPS, and presented frames) so diagnostic captures reflect live CPU activity.
  - **Diagnostic Crash Log Management**:
    - In `AppLog.kt`, clarified historical crash reports as `--- LAST CRASH REPORT (prior session) ---` and added an in-dialog button to clear the stored crash file once inspected.
- **Previous Release**: `v0.28.31-dev` (versionCode 71).
  - **Horizon OS NSO Memory Classification Fix for `rtld` Module Discovery**:
    - Corrected memory state classifications in `crates/memory/src/lib.rs` and `crates/loader/src/nso.rs`: set `.text` and `.rodata` segment memory states to `0x03` (`MemType_CodeStatic`) and `.data` / `.bss` to `0x04` (`MemType_CodeMutable`), matching exact Nintendo Switch Horizon OS kernel definitions.
    - Resolved critical infinite hang in Nintendo SDK runtime linker (`rtld`): `rtld` scans virtual address space via `svcQueryMemory` specifically filtering for `CodeStatic` (0x03) with `R-X` (5) permissions to discover non-rtld modules (`main.nso`, `subsdk*.nso`, etc.). The prior fallback of `0x02` (`MemType_Normal`) caused `rtld` to find zero modules and spin indefinitely without booting.
  - **Supervisor Call (SVC) ABI ID Alignments & System Primitives**:
    - Aligned `SVC_GET_SYSTEM_TICK` to official Horizon OS ID `0x1E` (was previously misplaced at `0x25`). Calculates 19.2 MHz Switch system ticks based on elapsed virtual nanoseconds.
    - Implemented `SVC_GET_PROCESS_ID` (0x24) returning 64-bit process ID.
    - Aligned `SVC_GET_THREAD_ID` to official ID `0x25` (was previously `0x29`).
    - Aligned `SVC_BREAK` to official ID `0x26` with backwards-compatible `0x2C` alias.
    - Implemented `SVC_SET_MEMORY_PERMISSION` (0x02) supporting memory protection updates for `.rodata` and dynamic code pages.
    - Implemented `SVC_GET_INFO` (0x29) supporting core masks, priority masks, alias region addresses/sizes, heap region address/size, total and committed memory sizes, ASLR regions, and deterministic entropy queries required by early nnSdk CRT initialization.
    - Updated `SVC_SLEEP_THREAD` (0x0B) to gracefully handle non-positive timeouts (`ns <= 0`, e.g. `0`, `-1`, `-2`) as thread yields rather than faulting.
  - **Activity Lifecycle & TextureView Shutdown Crash Guard**:
    - Fixed fatal uncaught `RejectedExecutionException` in `ProgramActivity.kt` (`onSurfaceTextureDestroyed`) during activity destruction: wrapped surface detachment and release tasks in `runWorkerTask` with `worker.isShutdown` checks and detached fallback execution, guaranteeing that `NativePrograms.setSurface(null)` and `surfaceTexture.release()` are always safely executed without crashing the main UI thread.
- **Previous Release**: `v0.28.30-dev` (versionCode 70).
  - **Horizon OS Futex Arbitration & Condvar Signaling Fixes**:
    - Fixed critical ABI register mapping in `SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC` (0x1C): aligned X0 (`mutex_addr`), X1 (`key_addr`), X2 (`tag`), and X3 (`timeout_ns`), eliminating inverted queue lookups that previously prevented `SignalProcessWideKey` from matching and waking condition variable waiters.
    - Enhanced `SVC_SIGNAL_PROCESS_WIDE_KEY` (0x1D) with proper mutex requeue semantics: when signaling condition variables, threads waiting on contested mutexes are requeued to the mutex FIFO queue with the contention flag (`0x4000_0000`) set, or granted immediate ownership if the mutex is unheld.
    - Updated `Waiter` and `FutexArbitrator` in `crates/kernel/src/sync.rs` with `tag: u32` preservation, ensuring correct owner thread tags are written to guest memory on `SVC_ARBITRATE_UNLOCK` and wake events.
    - Added dedicated test `test_condvar_wait_signal_requeue_cycle` in `crates/kernel/tests/svc.rs` validating multi-thread condvar wait, signal, mutex requeue, and ownership handover.
  - **Single-Region Fast-Path Memory Access**:
    - Added fast paths in `crates/memory/src/lib.rs` for `AddressSpace::read` and `AddressSpace::write` for single-region spans, avoiding iterator allocations and slicing overhead on every guest load/store.
- **Previous Release**: `v0.28.29-dev` (versionCode 69).
  - **Primed Dark Vulkan Presentation & Frame-Aware Boot Viewport**:
    - Eliminated uninitialized white screen on surface attachment by implementing `clear_initial_frame()` in `LiveRenderer` (`gpu_lab.cpp`), immediately acquiring, clearing to solid `#0a0e14` OLED black, and presenting an initial frame upon swapchain creation.
    - Implemented Android surface extent fallback query using `ANativeWindow_getWidth` / `ANativeWindow_getHeight` when `caps.currentExtent` returns `UINT32_MAX`, clamping dimensions to device-supported image extents and avoiding initialization panics.
    - Added structured `EmulatorGpu` Android logging via `<android/log.h>` across `LiveRenderer`, `program_set_surface`, and frame presentation paths for diagnostics.
    - Made `GameViewportView` frame-aware via `framesPresented`: retains authentic dark console display with scanlines, grid, and `"BOOTING GUEST RUNTIME…"` plus live instruction, core, and MIPS telemetry during early CRT/SDK initialization until real guest frames are delivered via `QUEUE_BUFFER`.
    - Added `FRAMES_PRESENTED` counter in `crates/android-ffi/src/program.rs`, incremented on each completed frame callback and reported in `emulator_program_step_v1` JSON.
- **Previous Release**: `v0.28.28-dev` (versionCode 68).
  - **Eliminated Cosmetic Waveform & Hooked Native Vulkan Display Presentation**:
    - Completely removed the synthetic oscilloscope / sine waveform animation placeholder from `ProgramActivity.kt` (`GameViewportView`).
    - Added 16:9 native display presentation surface (`GameTextureView`) embedded in `ProgramActivity`'s viewport hierarchy, backed by a rounded outline provider (`clipToOutline = true`).
    - Connected `ANativeWindow` lifecycle via `NativePrograms.setSurface(surface)`: automatically creates and tears down a `LiveRenderer` Vulkan swapchain targeting the Android presentation window.
  - **Real Framebuffer Delivery Pipeline via Maxwell 3D & Horizon BufferQueue**:
    - Implemented `ProgramGraphicsBackend` in `crates/android-ffi/src/program.rs`, bridging both `.nro` homebrew and retail `.nsp` program sessions with graphics capability.
    - Exported `emulator_program_set_frame_callback_v1` in `android-ffi`, allowing the engine to deliver rendered guest framebuffers synchronously to the C++ host renderer without large stack copies.
    - Implemented `present_frame` in `LiveRenderer` (`gpu_lab.cpp`), uploading guest framebuffers (RGBA/BGRA) into staging buffers and blitting them to the Vulkan swapchain with aspect-ratio letterboxing.
    - Connected `vi:m` BufferQueue `QUEUE_BUFFER` in `crates/services/src/vi.rs`, routing queued buffer presentations through `HosBinderDriver` to the active graphics backend.
    - Converted `GameViewportView` into a sleek translucent HUD and pause overlay: standby/ready state draws clean display background without fake animation, live execution displays transparent viewport with non-intrusive corner telemetry, and background/pause displays frosted overlay with prominent resume button.
- **Previous Release**: `v0.28.27-dev` (versionCode 67).
  - **Full Mobile Emulator GUI for Program Execution**:
    - Replaced the headless text-mode view in `ProgramActivity` with a full-fledged Switch emulator GUI.
    - Implemented 16:9 Switch display viewport canvas (`GameViewportView`) with 720p presentation (`1280×720 · 60Hz`), active CPU oscilloscope waveform, real-time instruction counters, execution rate telemetry (`MIPS`), program counter (`PC`), and simulated active core (`Core 0..2`).
    - Added on-screen touch controller (Virtual Gamepad) with Nintendo Switch diamond layout (**X**, **Y**, **A**, **B**), full 8-way directional D-pad, shoulder buttons (**L**, **ZL**, **R**, **ZR**), and system buttons (**－**, **＋**) with haptic feedback and show/hide toggle.
    - Added comprehensive emulator action bar: `⏸ Pause` / `▶ Resume`, `⏹ Stop`, `🔄 Restart`, `🎮 Controls` overlay toggle, and `📋 Logs` collapsible diagnostic console.
  - **Graceful Backgrounding, Pause & Resume Lifecycle**:
    - Replaced destructive session cancellation on `onPause()` with non-destructive session pausing.
    - Guest CPU registers, memory, kernel handles, threads, and scheduler state remain completely preserved in memory when putting the app in background or switching tasks.
    - On backgrounding or pausing, the viewport transitions to a frosted pause overlay displaying `⏸ PAUSED` and a prominent, tappable `▶ RESUME` button; returning to the app or tapping Resume immediately continues stepping from the exact instruction where execution was paused.
    - Preserved session continuity during guest synchronization waits (`state == "waiting"`) without prematurely dropping the session.
- **Previous Release**: `v0.28.26-dev` (versionCode 66).
  - **Phase 3 (M6A) Multi-Core Scheduler Simulation & Horizon Services**:
    - 3-core guest scheduler simulation (`active_core: 0..=2`), ideal core and core mask affinity tracking, priority-based round-robin core rotation.
    - Core affinity & priority SVCs: `SVC_SET_THREAD_PRIORITY` (0x0C), `SVC_GET_THREAD_PRIORITY` (0x0D), `SVC_GET_THREAD_CORE_MASK` (0x0E), `SVC_SET_THREAD_CORE_MASK` (0x0F), `SVC_GET_CURRENT_PROCESSOR_NUMBER` (0x10) returning current active core.
    - Shared memory mapping SVCs: `SVC_MAP_SHARED_MEMORY` (0x13), `SVC_UNMAP_SHARED_MEMORY` (0x14).
    - HID subsystem: 0x40000-byte shared memory ring layout for Touchscreen and Npad (Handheld, Player1..Player8) with real `SharedMemoryHandle` IPC relay on `IAppletResource::GetSharedMemoryHandle`, full vibration/controller command set (100–106, 120–126, 200–207).
    - Filesystem Proxy: RomFS hierarchical directory traversal (`DirectoryProxy`, `DirectoryEntry`), transactional SaveData commit stubs.
  - **Phase 4 (M6B) Maxwell 3D GPFIFO Engine & State**:
    - Complete B197 3D method state: 16 vertex streams (`0x1c00..=0x1cfc`), 32 vertex attributes (`0x1160..=0x11dc`), 16 viewports (`0xa00..=0xbf0`), 16 scissors (`0x1800..=0x18fc`), two-sided stencil test (`0x1384..=0x13a4`), independent blend targets (`0x1360..=0x137c`), blend equations (`0x1300..=0x133c`), MRT color targets, semaphores & syncpoints (`0x48..=0x5c`).
    - BC7 texture decompression (modes 0-7, mode 6 RGBA with 4-bit indices, modes 1-5, mode 7) & Tegra X1 TIC/TSC descriptor parsing.
  - **Phase 5 (M6C) Maxwell SM53 Shader Translator & Compute**:
    - Compute stage (`Stage::Compute`) entry points and SPH header parsing in SSA IR and SPIR-V generator.
    - SSA IR opcode coverage expanded: `Subtract`, `Fma`, `FMin`, `FMax`, `Sin`, `Cos`, `Exp2`, `Log2`, `Rcp`, `Rsqrt`, `Sqrt`, integer arithmetic (`IAdd`, `ISub`, `IMul`), bitwise (`BitwiseAnd`, `BitwiseOr`, `BitwiseXor`, `ShiftLeft`, `ShiftRight`), memory loads/stores (`ConstantBuffer`, `GlobalLoad`, `GlobalStore`, `SharedLoad`, `SharedStore`), workgroup barriers (`WorkgroupBarrier`).
    - CPU interpreter evaluator match arms for all new SSA IR opcodes.
  - **Phase 6 (M7A) Host Vulkan Immortalis-G720 Renderer**:
    - Automated compute-to-draw execution/memory barriers (`VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT` -> `VK_PIPELINE_STAGE_VERTEX_INPUT_BIT`).
    - Reverse-Z depth clamping (`depthClampEnable = VK_TRUE`) gated on `features.depthClamp`.
    - Extended dynamic state (EDS) driver fallback with specialized static graphics pipelines.
  - **Phase 7 (M7B) Android ARM64 Native Code Execution (NCE) Scaffolding & 16 KiB Page Alignment**:
    - Verified 16 KiB ELF alignment across `arm64-v8a` and `x86_64` native binaries.
    - All 22+ native engine symbols verified and exported in `android-ffi`.
- **Previous Release**: `v0.28.25-dev` (versionCode 65).
  - Implemented monotonic ticket futex arbitration in `crates/kernel/src/sync.rs` (`FutexArbitrator`) guaranteeing strict FIFO waiter order and immunity to spurious wakeups:
    - `SVC_ARBITRATE_LOCK` (0x1A): Atomic compare-and-swap lock acquisition with contention waiter tagging (`0x4000_0000`).
    - `SVC_ARBITRATE_UNLOCK` (0x1B): Wakes next queued waiter with ownership transfer.
    - `SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC` (0x1C): Atomically releases mutex and queues calling thread with monotonic ticket; re-acquires mutex upon wake.
    - `SVC_SIGNAL_PROCESS_WIDE_KEY` (0x1D): Wakes condition variable waiters in ticket FIFO order.
  - Implemented clean-room `nvnflinger` BufferQueue state machine in `crates/services/src/vi.rs`:
    - Full slot lifecycle (`Free`, `Dequeued`, `Queued`, `Acquired`) across `CONNECT`, `SET_BUFFER_COUNT`, `DEQUEUE_BUFFER`, `REQUEST_BUFFER`, `QUEUE_BUFFER`, `CANCEL_BUFFER`, `DISCONNECT`.
    - Android native window queries: `NATIVE_WINDOW_WIDTH` (1280), `NATIVE_WINDOW_HEIGHT` (720), `NATIVE_WINDOW_FORMAT` (1 RGBA_8888), `NATIVE_WINDOW_MIN_UNDEQUEUED_BUFFERS` (1), pending queued buffers count.
    - Extended `DisplayService` with layer commands (crop, alpha, position, size, indirect layers) and `GetDisplayVsyncEvent` (8225).
  - Enhanced NVIDIA Driver & GPFIFO Pipeline in `crates/services/src/nvchannel.rs` and `crates/services/src/nv.rs`:
    - Supported `NVIOC_CHANNEL_SUBMIT_GPFIFO_3` (`0xc000481b`) alongside `NVIOC_CHANNEL_SUBMIT_GPFIFO` (`0xc0004808`).
    - Added pointer-based `gpfifo` buffer resolution from GPU address space in addition to inline pushbuffers.
    - Added GPU Control ioctls (`/dev/nvhost-ctrl-gpu`): `GET_CHARACTERISTICS` (`0x80044705`) returning GM20B / Maxwell architecture metadata (arch `0x120`, impl `0x0b`, rev `0xa1`, 64 KiB big pages), ZCULL and ZBC stubs.
    - Supported read-only ioctls with receive-only buffers.
  - Added new Horizon System Services in `crates/services/`:
    - Account Service `acc:u0` / `acc:u1` / `acc:e` (`GetUserCount`, `GetUserExistence`, `ListAllUsers`, `OpenUser`, `GetLastOpenedUser`, `IProfile` with username "Player").
    - Performance Management `apm` / `apm:p` (`OpenSession`, `GetPerformanceMode` Handheld).
    - Audio Renderer and Output `audren:u` (`OpenAudioRenderer`, `GetAudioRendererWorkBufferSize`) & `audout:u` (`ListAudioOuts`, `OpenAudioOut` 48 kHz stereo 16-bit PCM).
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all Rust and Android unit tests.
- **Previous Release**: `v0.28.24-dev` (versionCode 64).
  - Lifted the artificial 2,000,000 instruction limit for packages in `emulator-android-ffi`, allowing retail games (including Pokémon Legends: Z-A) and NSP packages to run continuously.
  - Optimized step slice to 4096 instructions in `ProgramActivity.kt`, reducing JNI and serialization roundtrip overhead by 4x while maintaining smooth UI updates.
  - Implemented Phase 3 Horizon supervisor calls:
    - `SVC_SET_HEAP_SIZE` (0x01): Allocates, expands, or shrinks guest heap memory at `DEFAULT_HEAP_BASE` (`0x9000_0000`), page-aligned to 4 KiB, managing `AddressSpace` mappings with `MemoryState::Normal` (0x03) and Read/Write permissions.
    - `SVC_SET_MEMORY_ATTRIBUTE` (0x03): Handles memory caching and attribute hints.
    - `SVC_GET_CURRENT_PROCESSOR_NUMBER` (0x10): Returns active CPU core index.
    - `SVC_GET_SYSTEM_TICK` (0x25): Returns monotonic system tick counter derived from scheduler clock.
    - `SVC_GET_THREAD_ID` (0x29): Writes 64-bit thread ID to guest pointer for specified thread handle or current thread.
    - `SVC_BREAK` (0x2C): Captures breakpoint and assertion diagnostics in kernel debug log without halting execution unexpectedly.
    - Multi-thread futex synchronization: `SVC_ARBITRATE_LOCK` (0x1A), `SVC_ARBITRATE_UNLOCK` (0x1B), `SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC` (0x1C), `SVC_SIGNAL_PROCESS_WIDE_KEY` (0x1D) supporting non-deadlocking cooperative locking and condition variable waits.
    - Advertised newly implemented SVCs in `homebrew_config()` SVC capability mask.
  - Enhanced Horizon Visual Interface (`vi`) & presentation pipeline:
    - Implemented `HosBinderDriver` relay service with `TransactParcel` supporting `IGraphicBufferProducer` state machine (`CONNECT`, `REQUEST_BUFFER`, `DEQUEUE_BUFFER`, `QUEUE_BUFFER`, `CANCEL_BUFFER`, `QUERY`).
    - Expanded `DisplayService` to handle `GetRelayService` (100), `ListDisplays` (1000), `OpenDisplay` (1010), `CloseDisplay` (1011), `GetNaturalDisplayResolution` (1020), `CreateStrayLayer` (2020), `DestroyStrayLayer` (2030), `CreateManagedLayer` (2050), `DestroyManagedLayer` (2060), `SetLayerScalingMode` (2101), `ConvertScalingMode` (2102), `OpenSharedLayer` (5202).
    - Registered service port aliases: `vi:u` and `vi:s` -> `ViManagerService`, `nvdrv:a` and `nvdrv:s` -> `NvDriver`, `set` -> `SetSysService`.
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all Rust and Android unit tests.
- **Previous Release**: `v0.28.23-dev` (versionCode 63).
  - Resolved `unsupported guest SVC 0x6` encountered during retail NSO execution (Pokémon Legends: Z-A at instruction #1134).
  - Implemented 64-bit Horizon kernel supervisor call `SVC_QUERY_MEMORY` (0x06) adhering strictly to Horizon OS calling convention (register X0 = output MemoryInfo buffer in guest memory, register X2 = virtual address to query; returns X0 = 0 Result success, X1 = 0 PageInfo flags).
  - Added full virtual address space region query mechanism covering static codeset segments (R-X Code/Static, R-- RoData, RW- Data), Stack (0x0B), ThreadLocal (0x0C), Inaccessible (0x10), and unmapped gaps (0x00 Free).
  - Advertised `SVC_QUERY_MEMORY` capability in homebrew config mask.
  - Added option to clear earlier downloaded emulator APKs and updates in Settings under Storage.
  - Added explicit "Clear downloaded update" action in Updates screen when a verified update is ready.
  - Automatically cleans up staging and external download directory files to prevent APK accumulation.
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all Rust and Android unit tests.
- **Previous Release**: `v0.28.22-dev` (versionCode 62).
  - Resolved `unsupported A64 instruction 0xf81f0fe1 at 0x101b98` encountered during retail NSO execution (Pokémon Legends: Z-A at instruction #1132).
  - Corrected general-purpose load/store unscaled/pre-index/post-index decoding mask from `0x3f20_0800 == 0x3800_0000` to `0x3f20_0000 == 0x3800_0000`, eliminating the bit 11 mask restriction that previously prevented all pre-indexed load/store forms (`mode == 3`, such as `STR Xt, [sp, #simm]!`) from decoding.
  - Implemented SIMD & FP single-register unscaled immediate, pre-indexed, and post-indexed load/store operations under mask `0x3f20_0000 == 0x3c00_0000` across all operand sizes: 8-bit (B), 16-bit (H), 32-bit (S), 64-bit (D), and 128-bit (Q).
  - Implemented SIMD & FP single-register register-offset load/store operations under mask `0x3f20_0c00 == 0x3c20_0800` (`LDR / STR [Xn, Xm]`).
  - Added vector register accessors (`read_v_u8`, `write_v_u8`, `read_v_u16`, `write_v_u16`) with scalar zero-extension semantics adhering strictly to ARMv8 specification.
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all 37 A64 unit tests.
- **Previous Release**: `v0.28.21-dev` (versionCode 61).
  - Resolved `unsupported A64 instruction 0x1a9f17e8 at 0x100338` encountered during retail NSO execution (Pokémon Legends: Z-A at instruction #944).
  - Corrected conditional select decoding mask from `0x7fe0_0c00 == 0x1a80_0000` to `0x3fe0_0800 == 0x1a80_0000`, enabling full support for `CSINC`, `CSINV`, `CSNEG` and compiler aliases `CSET`, `CSETM`, `CINC`, `CINV`, `CNEG`.
  - Implemented A64 add/subtract with extended register `ADD`, `ADDS`, `SUB`, `SUBS` (`0x0b20_0000`) supporting all extend types (`UXTB`, `UXTH`, `UXTW`, `UXTX`, `SXTB`, `SXTH`, `SXTW`, `SXTX`), immediate shifts 0..=4, SP destination/source handling, and compiler aliases `CMP` / `CMN` with extended register.
  - Expanded load/store exclusive instructions to all operand sizes (8-bit, 16-bit, 32-bit, 64-bit) and added full acquire/release exclusive support (`LDAXR`, `STLXR`).
  - Implemented load-acquire and store-release non-exclusive memory operations `LDAR`, `STLR`, `LDARB`, `STLRB`, `LDARH`, `STLRH` (`0x0880_fc00`).
  - Implemented signed load pair instruction `LDPSW` (`opc == 1`) with 32-bit to 64-bit sign extension and corrected signed offset calculation in `LoadStorePair` and `FpLoadStorePair`.
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all 30 A64 unit tests.
- **Previous Release**: `v0.28.20-dev` (versionCode 60).
  - Resolved `unsupported A64 instruction 0xdac001ae at 0x1017d0` encountered during retail NSO execution (Pokémon Legends: Z-A at instruction #362).
  - Implemented A64 Data-processing (1 source) instructions: `RBIT` (reverse bits, 32-bit and 64-bit), `REV16` (reverse bytes in 16-bit halfwords, 32-bit and 64-bit), `REV` / `REV32` (reverse bytes in 32-bit words), `REV` (reverse bytes in 64-bit doublewords), `CLZ` (count leading zeros, 32-bit and 64-bit), and `CLS` (count leading sign bits, 32-bit and 64-bit).
  - Implemented ARMv8 architectural `HINT` instructions (including `NOP`, `YIELD`, `WFE`, `WFI`, `SEV`, `SEVL`, `BTI`, `PACIASP`, `AUTIASP`, `PACIAZ`, `AUTIAZ`, `PACIBSP`, `AUTIBSP`, `PACIBZ`, `AUTIBZ`, `PACIA1716`, `AUTIA1716`, `PACIB1716`, `AUTIB1716`, `XPACLRI`), memory barriers (`DMB`, `DSB`, `ISB`), and exclusive monitor clearing (`CLREX`) under mask `0xffff_e01f == 0xd503_201f`.
  - Expanded bitfield operations to support `BFM` (`opc == 1`), enabling `BFI` (Bitfield Insert) and `BFXIL` (Bitfield Extract and Insert Low) with destination register bit preservation.
  - Implemented register extraction instruction `EXTR` (`0x1380_0000` / `0x9380_0000`) for 32-bit and 64-bit concatenated extraction and rotation.
  - Implemented add/subtract with carry instructions `ADC`, `ADCS`, `SBC`, `SBCS` (`0x1a00_0000`) with full NZCV carry-in and borrow flag evaluation.
  - Implemented conditional compare instructions `CCMP` and `CCMN` (both register and immediate variants) under mask `0x3fe0_0410 == 0x3a40_0000` with condition evaluation and default immediate NZCV setting.
  - Expanded 3-source multiplications to support signed and unsigned wide multiplications `SMADDL`, `SMSUBL` (including `SMULL`, `SMNEGL`), `UMADDL`, `UMSUBL` (including `UMULL`, `UMNEGL`), and high-word multiplications `SMULH`, `UMULH`.
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all 26 A64 unit tests.
- **Previous Release**: `v0.28.19-dev` (versionCode 59).
  - Resolved `unsupported A64 instruction 0xd63f0020 at 0x1000bc` encountered during retail NSO execution (Pokémon Legends: Z-A).
  - Implemented A64 unconditional register branch instructions: `BLR Xn` (`0xd63f0000 | (rn << 5)`), `BR Xn` (`0xd61f0000 | (rn << 5)`), and `RET Xn` (`0xd65f0000 | (rn << 5)`).
  - Implemented test bit and branch instructions `TBZ` / `TBNZ` (`0x3600_0000` / `0x3700_0000`) for conditional branching based on tested bit values.
  - Implemented conditional select instructions `CSEL`, `CSINC`, `CSINV`, `CSNEG` (unlocking compiler aliases `CSET`, `CSETM`, `CINC`, `CINV`, `CNEG`).
  - Implemented integer multiply instructions `MADD`, `MSUB` (including `MUL`, `MNEG`), integer divide `UDIV`, `SDIV`, and register variable shifts `LSLV`, `LSRV`, `ASRV`, `RORV`.
  - Implemented logical immediate instructions with bitmask decoding `AND`, `ORR`, `EOR`, `ANDS`.
  - Expanded memory load/store operations to support 8-bit (`STRB`, `LDRB`, `LDRSB`), 16-bit (`STRH`, `LDRH`, `LDRSH`), 32-bit (`STR`, `LDR`, `LDRSW`), 64-bit (`STR`, `LDR`), unscaled `LDUR` / `STUR`, pre/post-indexed addressing, and register offset addressing `[Xn, Xm]`.
  - Corrected retail NSO startup registers in `Session::from_exefs_with_services` to initialize `x0 = 0`, `x1 = MAIN_THREAD_HANDLE` (0x100), and write `MAIN_THREAD_HANDLE` to `TLS_PAGE + 0x110` according to Nintendo Horizon OS process launch conventions.
- **Previous Release**: `v0.28.18-dev` (versionCode 58).
  - Resolved `invalid loader input: failed to parse NSO module 'rtld': invalid loader input: NSO section SHA-256 hash mismatch` when loading commercial Switch NSP packages (including Pokémon Legends: Z-A).
  - Corrected NSO0 section SHA-256 hash offsets in the header from `0x70 + i * 0x20` to `0xA0 + i * 0x20` (Text hash at 0xA0..0xC0, RoData hash at 0xC0..0xE0, Data hash at 0xE0..0x100) per Nintendo Horizon NSO0 header specifications.
  - Properly authenticated decompressed section digests against header digests when hash checking flags (bits 3, 4, 5) are active.
  - Added unit test coverage validating SHA-256 hash calculation and verification at offset 0xA0, as well as rejection of corrupted section payloads.
- **Previous Release**: `v0.28.17-dev` (versionCode 57).
  - Resolved `invalid loader input: ExeFS contains no valid NSO modules` on commercial Switch NSP game packages.
  - Fixed NSO0 header flags offset to `0xC` (per Horizon NSO0 format specification; retaining `0x8` for backwards compatibility) ensuring compressed retail NSO sections are properly recognized and decompressed with LZ4.
  - Removed strict page-multiple restriction on NSO section decompressed sizes, properly page-aligning memory regions during load.
  - Increased maximum image file size to 256 MiB and memory size to 512 MiB to accommodate commercial game executables.
  - Added path normalization (`Pfs0Archive::normalize_filename`) stripping leading slashes, `./`, `exefs/`, nulls, and whitespace during file extraction and lookup.
  - Expanded ExeFS candidate executable names (`rtld`, `nnrtld`, `main`, `program`, `application`, `sdk0`, etc.) and prevent silent suppression of NSO parsing errors during fallback archive scanning.
  - Updated `open_exefs()` so non-executable logo partitions (containing `NintendoLogo.png` / `StartupMovie.gif`) do not overwrite or mask primary ExeFS decryption errors.
  - Enhanced ExeFS error reporting to detail archive file inventory (filenames, sizes, magic headers) when no valid modules are detected.
- **Previous Release**: `v0.28.16-dev` (versionCode 56).
  - Resolved `invalid loader input: invalid PFS0/HFS0 magic` on Hierarchical SHA-256 PartitionFS (ExeFS) and CNMT sections.
  - Implemented `NcaFsHeader::data_region()` to parse data layer offsets from `HierarchicalSha256Data` (skipping layer 0 SHA-256 hash block tables) and IVFC levels.
  - Updated `open_exefs()` and `parse_cnmt()` to slice decrypted payloads by `data_region()` to locate the inner PFS0 archive.
  - Updated `decrypt_section()` candidate key testing to verify magic at `data_region.offset` using the corresponding offset CTR counter.
  - Added fallback in `ExeFsProgram::parse()` and `parse_cnmt()` to scan for `PFS0`/`HFS0` at 0x200-aligned sector boundaries in un-sliced payloads.
- **Physical Device Telemetry (Issue #15)**:
  - Latest report: `1794318272960-44c3d786`, app version `0.28.6-dev` on `Mali-G720-Immortalis MC12`.
  - Diagnostics: 269/275 checks passed, 65/66 visible patterns passed.
  - Normalized-position candidate path passes all 23 checks (`normalized-position-texture-*` and `normalized-inset-*`) and all 11 candidate visible patterns (Variants 55–65) with 0 mismatched pixels.
  - Issues #5–#15 (0.27.6-dev through 0.28.6-dev) all have the same strict result. The baseline projected texture and five controls fail; candidate passes. Four baseline pixel mismatches match colors predicted by captured UVs. No new version-specific GPU regression is confirmed.

---

## Delivered Milestones

1. **Loader & File Formats**:
   - Clean-room PFS0 partition archive parser in [`crates/loader/src/pfs0.rs`](file:///root/projects/emulator/crates/loader/src/pfs0.rs).
   - NPDM metadata parser in [`crates/loader/src/npdm.rs`](file:///root/projects/emulator/crates/loader/src/npdm.rs) (`META` header, Title ID, product code, main thread priority, core affinity, stack sizing).
   - ExeFS program loader in [`crates/loader/src/exefs.rs`](file:///root/projects/emulator/crates/loader/src/exefs.rs) (sequential loading and mapping of `main.npdm`, `rtld.nso`, `main.nso`, `subsdk*.nso`).
   - Dynamic ELF relocation engine in [`crates/loader/src/reloc.rs`](file:///root/projects/emulator/crates/loader/src/reloc.rs) (`R_AARCH64_RELATIVE`).

2. **Horizon OS Services**:
   - `sm:` Service Manager registration and dispatch.
   - `set:sys`: `GetLanguageCode` (AmericanEnglish), `GetRegionCode` (USA) in [`crates/services/src/setsys.rs`](file:///root/projects/emulator/crates/services/src/setsys.rs).
   - `time:s` / `time:u`: Steady clock `GetCurrentTime` in [`crates/services/src/timesys.rs`](file:///root/projects/emulator/crates/services/src/timesys.rs).
   - `fsp-srv`: Filesystem proxy service (RomFS, SaveData mounts, `IFileSystem` proxy) in [`crates/services/src/fspsrv.rs`](file:///root/projects/emulator/crates/services/src/fspsrv.rs).
   - `vi:m`: Visual interface manager (`GetDisplayService`, `OpenDisplay`, `GetNaturalDisplayResolution` 1280x720, `CreateStrayLayer`) in [`crates/services/src/vi.rs`](file:///root/projects/emulator/crates/services/src/vi.rs).
   - `appletAE` / `appletOE`: Applet manager (`GetAppletResourceUserId`, `GetOperationMode`, `GetPerformanceMode`) in [`crates/services/src/applet.rs`](file:///root/projects/emulator/crates/services/src/applet.rs).
   - `hid`: Human Interface Device service (`CreateAppletResource`, `ActivateDebugPad`, `SetSupportedNpadStyleSet`, `SetSupportedNpadIdType`, `ActivateNpad`) in [`crates/services/src/hid.rs`](file:///root/projects/emulator/crates/services/src/hid.rs).

3. **AArch64 Execution Engine**:
   - PC-relative addressing (`ADR`, `ADRP`).
   - Logical shifted register operations (`AND`, `BIC`, `ORR`/`MOV`, `ORN`, `EOR`, `EON`, `ANDS`, `BICS`).
   - Bitfield extraction and sign extension (`UBFM`, `SBFM`).
   - Atomic exclusive load/store (`LDXR`, `STXR` 32-bit and 64-bit).

4. **NCA3 Metadata & Section Decryption Engine**:
   - Clean-room decrypted main-header parser in `crates/loader/src/nca.rs`.
   - Parses distribution/content fields, key-generation metadata, program/rights IDs, four indexed section entries, encrypted key area (`0x300..0x340`), and stored filesystem-header hashes.
   - Filesystem section header parser (`NcaFsHeader`) reading version, filesystem type, hash type, encryption type, and 8-byte section counter (`section_ctr`).
   - Verified section filesystem header SHA-256 integrity checks against main header stored hashes via `verify_fs_header_hashes()`.
   - Big-endian AES-CTR counter generation via `make_section_ctr` using the format-specific 8-byte reversed section counter prefix and media offset block index.
   - Section payload decryption via `decrypt_section()` supporting title-key content keys and key-area unwrap via AES-128-ECB. Sixteen NCA unit and integration tests pass. Details: [`docs/nca-parser.md`](docs/nca-parser.md).

5. **Crypto foundations**:
   - Clean-room dependency-free `emulator-crypto`: AES-128 block encrypt/decrypt, ECB decrypt, explicit-counter CTR, full-block XTS, FIPS 180-4 SHA-256 digest engine, and bounded `name = hex` key-file parsing with redacted debug output.
   - AES-128 FIPS 197, CTR SP 800-38A, XTS known-answer, and SHA-256 NIST test vectors pass. Lookup-table AES is not constant-time and is not production-hardened.

6. **Phase 1 (M5A) Delivered - NCA3 Decryption, RomFS & ExeFS Integration**:
   - Clean-room RomFS parser supporting hierarchical directories, file tables, and IVFC (Hierarchical Integrity) level extraction in `crates/loader/src/romfs.rs`.
   - Clean-room LZ4 block decompression in `crates/loader/src/lz4.rs` for retail compressed NSO modules.
   - NSO loader now handles compressed `.text`, `.rodata`, `.data` sections and verifies SHA-256 digests.
   - Multi-module ExeFS program loading and dynamic relocation support integrated with NPDM stack sizing in `Session::from_exefs`.
   - Full NCA3 container execution in `Session::from_nca`: header XTS decryption, integrity verification, ExeFS module loading, and RomFS mounting into `fsp-srv`.
   - CLI command `run-nca` in `crates/cli` for running decrypted/encrypted NCA game programs directly.

7. **Phase 2 (M5B) Delivered - AArch64 Floating Point & NEON Vector Engine**:
   - Extended `Cpu` with 32 128-bit vector registers `v: [u128; 32]`, `fpcr`, and `fpsr`.
   - Scalar FP arithmetic: `FADD`, `FSUB`, `FMUL`, `FDIV`, `FMAX`, `FMIN`, `FMAXNM`, `FMINNM`, `FNMUL`.
   - Scalar FP unary ops: `FMOV`, `FABS`, `FNEG`, `FSQRT`.
   - Fused multiply-add/sub: `FMADD`, `FMSUB`, `FNMADD`, `FNMSUB`.
   - Float comparison: `FCMP` (register and zero), updating architectural NZCV condition flags.
   - Conversions: `FCVT` (f32 <-> f64), integer conversions (`FCVTZS`, `FCVTZU`, `SCVTF`, `UCVTF`), and general/FP moves (`FMOV`).
   - SIMD & FP memory operations: `LDR` / `STR` for S (32-bit), D (64-bit), and Q (128-bit) registers with immediate scaling.
   - SIMD & FP pair operations: `LDP` / `STP` for S, D, and Q registers with pre/post-indexing.
   - NEON 128-bit vector bitwise logic: `AND`, `BIC`, `ORR`, `ORN`, and `EOR`.

8. **NSP Package Container & Decryption Engine**:
   - Clean-room NSP (Nintendo Submission Package / PFS0) parser in [`crates/loader/src/nsp.rs`](file:///root/projects/emulator/crates/loader/src/nsp.rs).
   - Ticket (`.tik`) parser in [`crates/loader/src/ticket.rs`](file:///root/projects/emulator/crates/loader/src/ticket.rs) supporting RSA-2048, RSA-4096, and ECDSA signature types.
   - Common title key unwrapping via AES-128-ECB decryption using `titlekek` matching master key revision; dynamic key registration into `KeyFile`.
   - CNMT (`PackagedContentMeta`) parser in [`crates/loader/src/cnmt.rs`](file:///root/projects/emulator/crates/loader/src/cnmt.rs) extracting Content IDs and mapping Program/Control/LegalInformation NCAs from `.cnmt.nca`.
   - Full package execution via `Session::from_nsp`: ticket import, CNMT program resolution, NCA3 header XTS decryption, ExeFS CTR decryption, RomFS mounting, and guest code execution.
   - CLI command `run-nsp` in `crates/cli` with `--keys` and `--titlekeys` support. Details: [`docs/nsp-parser.md`](docs/nsp-parser.md).

---

## Canonical Pokémon Legends: Z-A Execution Roadmap

The complete clean-room master plan for booting and running Pokémon Legends: Z-A on the Realme GT 7 Dream Edition (Arm Immortalis-G720) is documented in [`docs/pokemon-legends-za-plan.md`](file:///root/projects/emulator/docs/pokemon-legends-za-plan.md).

The implementation roadmap is structured into eight concrete milestones:
1. **Phase 1 (M5A) - NCA3 Decryption, Key Hierarchy & RomFS**: [DELIVERED]
2. **Phase 2 (M5B) - AArch64 Floating Point & NEON Vectors**: [DELIVERED]
3. **Phase 3 (M6A) - Horizon OS Multi-Core Kernel & Services**: 3-core guest scheduler simulation, user-space futex arbitration (`svcWaitProcessWideKeyAtomic`), full `fsp-srv`, `vi:m`/`nvnflinger` BufferQueue presentation, `hid` controller ring buffers, `audren:u` audio mixing, and `nvdrv` GPFIFO submission.
4. **Phase 4 (M6B) - Maxwell 3D GPFIFO Engine & State**: GPFIFO pushbuffer parsing (methods, gather buffers, call/return), full B197 3D state machine (16 VBs, 32 attrs, two-sided stencil, MRTs, independent blend), TIC/TSC samplers, and syncpoint semaphores.
5. **Phase 5 (M6C) - Maxwell SM53 Shader Translator & Compute**: Full SM53 opcodes, typed SSA IR, compute workgroup shared memory (`LDS`/`STS`), `BAR.SYNC`, perspective-correct fragment interpolation (`IPA`), and SPIR-V 1.3 emission.
6. **Phase 6 (M7A) - Host Vulkan Immortalis-G720 Renderer**: Compute-to-draw skinning barriers (preventing post-menu black screens), EDS capability-gated static pipeline fallback, GPU compute BCn decompression to ASTC/RGBA with retained GOB backing, and Reverse-Z depth clamping.
7. **Phase 7 (M7B) - Android ARM64 Native Code Execution (NCE)**: Host direct execution on Dimensity 9400e Cortex cores, 16 KiB-aligned virtual address space, and fast SVC signal interception (>60 MIPS/core).
8. **Phase 8 (M8) - Pokémon Legends: Z-A Acceptance Campaign**: Staged verification (Title Boot -> Menu -> Post-Menu Intro -> Lumiose City 3D Exploration -> Save/Load Durability at 30 FPS).

---

## Next Planned Steps (In Exact Priority Order)

1. **Horizon VI BufferQueue & Presentation Pipeline (Phase 3 / M6A)**:
   - Implement clean-room `nvnflinger` BufferQueue state machine (`Connect`, `RequestBuffer`, `QueueBuffer`, `DequeueBuffer`).
   - Connect buffer presentation to the Android Vulkan swapchain / `ANativeWindow`.

2. **NV Channel GPFIFO Submission Pipeline (Phase 4 / M6B)**:
   - Support `NVIOC_CHANNEL_SUBMIT_GPFIFO` command buffer submission.
   - Wire guest command stream decoding directly into the host Vulkan command processor.

3. **Multi-Thread Futex Synchronization (Phase 3 / M6A)**:
   - Implement monotonic ticket arbitration for `svcWaitProcessWideKeyAtomic` and `svcSignalProcessWideKey` preventing post-menu deadlock in Trinity Engine.
