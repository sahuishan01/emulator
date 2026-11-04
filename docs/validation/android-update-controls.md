# Manual update controls and access feedback (0.27.3-dev)

The manual-check button previously disappeared without a saved token. Connection
feedback was above the token form and release notes, so it could be off-screen
after the dialog closed. Library → Updates and the early Settings → App updates
entry now lead to visible **Check for updates** and **Test GitHub access** actions.
Both offer token connection when needed. Access status, last verification time,
last successful release check and the current result appear above the controls.
Connection scrolls to the result; release notes appear after configuration.

All 27 JVM tests pass, including separate authentication/repository errors,
primary and Retry-After rate limits, and errors that never echo a response body
or credential. Existing metadata, size/hash, redirect, cancellation and credential
restriction tests still pass. Pinned Android assembly, test-package assembly and
lint pass. No runtime dependency or permission is added.

Fourteen device instrumentation checks pass through the actual UpdatesActivity
and manager, using a QA-only injected network and synthetic token. They cover
disconnected controls, masked/empty input, rejected tokens not saved, explicit
successful access while up to date, manual checks with automation disabled and
within the background throttle, access tests with an available release that never
download, in-flight button disabling, offline failures retaining history, denied
saved access and retry, failed replacement preserving the previous token/state,
manager recreation preserving verification/check times, landscape recreation,
disconnect clearing history, and migration of an existing unverified saved token.
Seven screenshots retain the disconnected, rejected, verified, offline, denied,
restored and landscape states. This verifies feedback behavior; no real user's
GitHub credential was placed on the emulator or used by these tests.

Seven separate adb-driven checks pass on the final APK. A real signed update from
0.27.2 to 0.27.3 preserves every library row and the exact existing diagnostic
cache. Library and Settings navigation, both disconnected actions, masked input
and empty-token rejection, cancellation without storing access, early Settings
placement, landscape reachability and cold-launch controls all pass. Screenshots
and taps use fresh UI trees. Diagnostics remain immutable; no graphics rerun is
needed for this UI change. Both native libraries match 0.27.2 byte-for-byte, keeping
the same 252 diagnostics/55 patterns, schema 4/workload 31/native 16 and 21 Rust exports.
The physical Mali inset issue remains open.

The initial adb run encountered an unrelated **System UI isn't responding** boot
dialog after successfully checking installation preservation. Its raw evidence
is retained. The harness now handles only that exact system dialog with **Wait**,
records it and continues; target-app failures remain fatal. The rerun records
all seven checks from 0.27.2 to 0.27.3. An initial instrumentation attempt stopped
after one check because its dialog helper compared mixed case with Android's
uppercase button text. The helper was corrected, only the test package rebuilt,
and all 14 checks rerun. The production APK stayed byte-identical during this
harness correction. No app acceptance requirement was relaxed.

Final crash buffers are empty, without a target-app fatal exception, native fatal
signal or ANR. The fake credential and its AtomicFile backup are absent after
tests, verification preferences are cleared, and the test package is uninstalled.
Success/rejection and disconnected/library screenshots were visually inspected.
The owned API35 AVD started after 98.13% CPU idle, nice 10/two guest cores,
2048 MiB requested (raised to 2560 MiB). It used the existing 720x1280/density 280
override; size/density were not changed in this turn. Orientation was restored,
the AVD explicitly stopped and its process awaited with exit 0; ADB is empty.
Evidence remains ignored under reports/v0.27.3-* and
reports/update-qa/v0.27.3-dev/.

App version/code: 0.27.3-dev/33. APK size: 12,108,988 bytes. SHA-256:
`35cb92b28b54b87f2c5034b09c50335e3e6a96acecc3e45a1cf83001f35b19f8`.
Signature, the previous signing identity, both native architectures, 21 exports,
16 KiB ELF/ZIP alignment, the exact existing permissions/components and QA-code
exclusion pass. Packaging alignment is not a 16 KiB runtime-device claim.
