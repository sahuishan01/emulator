# Positive-W projection validation (0.27.0-dev)

The supplied [physical 0.26 Mali report](g720-depth-clear.md) confirms all 208
previous diagnostics and 39 ordered patterns, including every depth-clear case.
This checkpoint adds [sixteen projected rendering scenarios and sixteen portable
groups](../projection-batch.md). Physical 240/55 confirmation remains pending.

All 267 workspace Rust tests pass. Formatting and all-target Clippy with warnings
denied pass; the 17 new targeted tests also pass after the final iterator cleanup.
The 16 portable groups report 167 verified cases/cycles, including 64 complete
resource cycles and rejection, rollback, ownership, snapshots and read-only
behavior. All 31 unchanged independent LLVM fixtures verify. The sixteen new
command pages and all older depth/depth-clear pages reproduce from our generators.
No existing emulator core or new shader encoding was introduced.

Fresh host engine 18/NV 50/GPU core 76/shader 25 reports pass. Every previous
208 named test record is unchanged. All 71 host Vulkan checks pass with
validation enabled, zero errors and an empty error field. Native Vulkan executes
actual fixed-function projection from captured clip-space vertices; complete
color and, for depth cases, D32 readback match independent coordinate formulas.
All 71 Android native test records match the host records exactly.

Pinned two-ABI Android assembly and lint pass, together with signature, ELF
architecture, all 21 unchanged exports, 16 KiB ELF/ZIP alignment and strict
main-app manifest checks. App/code/schema/workload/native versions are
0.27.0-dev / 30 / 4 / 30 / 15. APK size is 16,773,825 bytes, SHA-256
`dbe3ccbcbf76fb60f669036820c83e3e9c81ed33472e46698095337dcb217453`.
The package and signing certificate match 0.26 for updates. No permission is
added. Packaging alignment is not a 16 KiB Android runtime test.

Automatic Android QA passes all 14 workflow checks with two fresh completed
240-diagnostic/55-pattern runs. All 55 captures verify 7,040 displayed pixel
centers in the first run, without recapture. Cancellation, upgrade/catalog/cache
preservation, sharing, rotation, background restart, cold restore and exactly
one published Downloads JSON per completed run pass. Exported bytes equal the
private cache and MediaStore metadata is correct. An unrelated System UI startup
Wait was handled explicitly. Android validation layers were disabled; host
validation coverage is recorded separately above.

Manual QA passes 63 flows at native resolution. Fifty-nine successful captures
verify 7,552 displayed pixel centers across every pattern, wrap, rapid changes,
rotation, background/foreground and repeated native/surface teardown. The initial
55 zero-duration injected taps ended at variant 53 instead of 0 and timed out.
The original trace, scene report, screenshot, XML and logs are retained. A
separate 55-press probe with 80 ms stationary contact returned to its starting
variant exactly. The resumed control/lifecycle test uses that contact duration,
keeps the same 55-press/final-variant requirement, and passes. Navigation to the
last pattern uses another checked 54-press burst instead of repeating all pixel
captures. No app rendering code, pixel expectation or press count was relaxed.
The underlying cause of unreliable zero-duration injection is not conclusively
identified; this checkpoint does not claim to fix it. AOSP's
[input shell implementation](https://android.googlesource.com/platform/frameworks/base/+/master/services/core/java/com/android/server/input/InputShellCommand.java)
shows identical timestamps for tap down/up. Raw evidence includes
manual/nonzero-press-probe.json and the failed zero-duration trace.

Standard export/recovery QA resumes the actual completed automatic report and
passes all 240 cases, four full Rust-reference comparisons and 71 exact native
records. The visible summary, real picker export, cold restoration and both
present AtomicFile cache recoveries are byte-exact. There is no current-version
Catch cache, so a third recovery is not claimed. Five empty UI-dump attempts
needed bounded fresh-XML retries. The original automatic JSON remains immutable
after manual scenes and Diagnostics reopening. All three final crash buffers
are empty, without target-app fatal exception, native fatal signal or ANR.

The owned AVD started after 98.28% CPU idle, nice 10/two guest cores/2048 MiB
requested (raised to 2560 MiB by the emulator). Boot took 82.7 seconds; the initial
50-second readiness check expired before boot was confirmed and retried.
Automatic captures used 720x1280/density 280, reset to native 1080x1920/density
420 before manual QA. The owned AVD was shut down, its process awaited with
exit 0 and ADB confirmed empty. No physical ADB target was used.

The projected rectangle, right-scissored texture and divided-depth captures were
visually inspected. Raw reports/captures remain ignored under reports/v0.27-*
and reports/android-qa/v0.27.0-dev/. Smooth perspective-correct attributes,
clipping, larger depth layouts, stencil, general runtime/display and commercial
games remain open.
