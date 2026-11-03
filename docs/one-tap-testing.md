# One-tap testing and JSON export (0.22.1)

The [0.24 depth extension](depth-batch.md) expands the same workflow to
21 patterns and 168 diagnostics. Current reports use app 0.24.0-dev/code 27,
schema 4/workload 27/native 12. The historical baseline below describes the
original UI/lifecycle contract; current pattern/group lists are dynamic.

The [0.23 storage extension](texture-storage-batch.md) expands the same workflow
to12 patterns/151 diagnostics. The0.22.1 baseline and lifecycle contract below
remain applicable; new reports use app0.23/code26/schema4/workload26/native11.

The home screen's **Test all & export JSON** button opens a private activity and
starts the complete workflow. It displays all six owned patterns in order, runs
the existing 134 diagnostic checks, and saves a JSON report in **Downloads/Emulator**.
The completed screen lists pattern results and offers **Share JSON** and **Run again**.
Sharing opens the system chooser; saving does not require selecting a destination.
Manual scene controls and Settings → Diagnostics remain available.

## Execution and report

One HandlerThread owns all scene/Vulkan calls, then all five diagnostic groups.
Each pattern must have native pixel verification, one completed/presented frame,
zero reported validation errors, an empty error and a TextureView surface-update
callback. A successful pattern stays visible for approximately 1.5 seconds before
the next begins. The callback is consumer evidence, not a camera/display measurement.
Native validation-layer availability is still reported separately by the Vulkan
group. GPU byte verification uses the existing independent scalar pixel oracles.

Pattern failures are recorded and the other patterns/checks continue. The report's
`patternBatch` holds the unique run ID, start/end times, six named native snapshots,
surface-update observations, per-pattern pass/error fields and totals. Root
`allPassed` requires both the complete pattern batch and all diagnostic groups.
The corpus remains 134 checks; the six visible patterns are separately counted.
Unexpected process/native-driver termination can still prevent completing a report.

`Diagnostics` owns the common native bindings/report builder for both automatic
and manual flows. `NativeScenes` is shared with the manual TextureActivity; JNI
delegates and all 18 Rust value exports retain their underlying behavior. The
app is 0.22.1-dev/code25, report schema4/workload25/native10. No Rust emulation
or shader capability was added in this UI update.

## Cancellation and lifecycle

Cancel invalidates queued results and publishes no new report. An in-progress
native call finishes before its thread can process teardown. Backgrounding,
surface loss or a size change during an unfinished run cancels that attempt;
foreground availability starts a fresh complete run. Results from abandoned
generations cannot publish or join the new run. Destroyed SurfaceTextures remain
retained until native teardown on their owning thread.

The brief export phase completes using the application context, including when
backgrounded. When a complete cached report is available, activity restoration checks its run ID and that its saved
URI is published; it restores a completed result or offers Retry export for an
interrupted save. Completed orientation changes and sharing do not create a new
export. The completed screen replaces the stopped scene with a scrollable result
list, so recreating a completed surface does not imply rerunning the suite.

Manual Diagnostics treats an automatically published report as immutable when
resumed. It still reads/recovers ancillary AtomicFile caches, but does not
merge later scene snapshots into the already exported JSON. A new manual run
creates a new diagnostic report with current captures.

## Downloads publication

The exporter uses Android's
[MediaStore.Downloads API](https://developer.android.com/reference/android/provider/MediaStore.Downloads)
for an app-owned JSON file on the supported API31+ devices. It adds no storage or
network permission. A unique timestamp/random-suffix filename preserves older
reports. The URI/filename/directory are included in `automaticExport`.

The MediaStore row starts pending. The exact UTF-8 JSON is bounded to 1MiB and
saved using the existing private AtomicFile cache, then written/closed in the
Downloads entry. Only then is the row published. Exceptions attempt to remove
the pending entry and leave a visible Retry export action. A process death can
leave an unpublished pending row for Android's own cleanup. Cache and Downloads
publication are coordinated, not a cross-provider atomic transaction.

Share grants temporary read access to that content URI and opens the Android
chooser. The app does not select a recipient or transmit the report automatically.
See [validation evidence](validation/android-one-tap.md) for current coverage.
