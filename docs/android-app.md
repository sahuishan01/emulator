# Android application (0.20 foundation, 0.22.1 automatic testing)

The launcher is a library, with the original Catch player, the 0.21 guest texture scene, supported static
NRO imports, text-mode execution, recents and settings. The home screen also offers
[one-tap patterns, diagnostics and JSON export](one-tap-testing.md) through
**Test all & export JSON**, saving into Downloads/Emulator. The existing laboratory
is a private support activity under Settings → Diagnostics. The application ID
and debug signing identity remain unchanged so this development build can update
an existing installation. No commercial game compatibility is implied.

## Ownership

`EmulatorApplication` owns `LibraryStore`. One worker serializes catalog/file
transitions; activities observe status while started and remove their observers
when stopped. An import continues through activity recreation. Import state uses
the application context instead of storing its originating activity, and does not
depend on a picker URI remaining available after import.

SQLite schema 1 stores an opaque ID, display title, private filename, byte count,
mode, last-played time, launch count and last result. Catch has a protected built-in
entry; the texture scene is also protected and is seeded idempotently into existing catalogs. Imported entries use the full file SHA-256. Metadata is separate from guest
memory and does not constitute a save state. Search matches display titles locally.
Missing private files disable launch and explain how to recover.

## Import contract

The system Storage Access Framework picker grants the selected document only.
The main APK requests no network or broad storage permission. Imports copy into
the app's private directory in 32 KiB chunks, hash the exact bytes, sync the file,
and ask the original Rust NRO parser to validate it before publication. A successful
rename is followed by a catalog insert. Failed inserts delete the copied file;
process-start cleanup removes partial imports and unindexed digest files.

Limits are 32 MiB/file, 256 MiB/library and 128 imported entries. Display names
are normalized/bounded and never used as paths. Private filenames must be 64 lower
hex digits plus `.nro`. Exact duplicates and the byte-identical included Catch are
recognized. Bad headers, unsupported module headers/flags, over-limit files and
partial copies do not become library entries. A valid static format does not imply
supported instructions/services. Remove deletes the app-private copy and row;
it never deletes the provider's source document.

Cancel marks the current job, sends the provider cancellation signal and closes
its stream. Partial files are removed before the final status is published. A
provider that ignores cancellation and blocks indefinitely can still delay this
single worker; imports are not a persistent background download service. Process
death aborts an in-flight import; the next start cleans up its temporary file.

## Execution contract

`NativePrograms` provides inspect/start/step/stop through JNI. C++ reads only the
resolved private file and uploads bounded 4096-byte value chunks; no guest pointer
or Java buffer crosses into Rust. The six new C exports use the existing bounded
`EngineReport` plus a 4100-byte `ProgramChunk`, with layout assertions in Rust/C++.
Each calling thread owns its upload/session; close is idempotent.

Imported programs execute through the actual original `Session`, interpreter,
kernel and services. Each Android slice is 1024 instructions; the ABI accepts
1–4096 and caps the session at 2,000,000 instructions. Exit, faults, execution
limits and waiting for events are explicit results. Waiting ends this text runner;
external event delivery/resume is not implemented. Output is bounded to 512 Unicode
characters from at most 2048 raw bytes, with truncation reported. IPC traces are
drained at slice boundaries; exit/fault/limit/wait releases the session.

The activity owns one execution worker, checks cancellation between slices,
stops when backgrounded, and closes on that same worker. It shows real debug
output and permits copying it. Imported programs do not yet present general
homebrew graphics. Catch continues to use its dedicated original live Vulkan path.

## Player, settings and reports

Catch supports drag, held Left/Right, D-pad, Pause/Resume and Reset. Slow/Normal/Fast
held-repeat intervals are 120/80/50 ms after a 240 ms initial delay. Screen-awake
and optional frame/GPU details are persisted locally. Touch cancellation and
activity/surface teardown retain the previous ownership rules. Backgrounding ends
the current Catch session; returning starts a new one. There is no save-state or
background in-memory resume feature.

The library uses a full-page scroll in landscape; program output/settings scroll
in either orientation. The only exported activity is HomeActivity. The diagnostics
now run all 134 cases and export one report. The diagnostics, Catch and texture-scene caches use synchronized
AtomicFile writes and bounded reads, including backup recovery. No QA document
provider or its fixtures ship in the main APK.

## Remaining production work

The [texture scene](texture-draw-batch.md) presents actual sampled guest shader output, supports six variants including three indexed-geometry patterns, recreates on rotation/foreground and skips superseded queued work. Its support snapshot joins the ordinary report export. Imported programs still use the text runner.

This is the application foundation, not production readiness. Broader CPU/runtime,
module loading, normal HID/VI/BufferQueue, general graphics/shaders, audio,
save-state semantics and commercial content remain open. Device performance and
compatibility require actual workloads on the target GPU. Store distribution also
requires a deliberate release-signing/migration policy and release qualification;
this iteration remains a private, debug-signed development prerelease.
