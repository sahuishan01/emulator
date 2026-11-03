# In-app updater validation (0.27.1-dev)

The updater keeps GitHub releases private and uses per-device access entered by
the user. No real host GitHub token was copied into the APK or test device.
The supplied [Mali report](g720-projection.md) has one projected inset-texture
failure (239/240 checks and54/55 patterns); the updater preserves that result.
The GPU/CPU code and native libraries are unchanged from0.27, byte for byte.
This release does not fix or hide the rendering mismatch.

Twenty-four JVM tests pass for semantic version ordering and prereleases,
release selection, malformed/missing APK metadata, checksums, bounds, truncation,
corruption, cancellation, approved URLs, redirects, credential removal before CDN
requests, access/rate/server errors, metadata limits and connection teardown.
Twenty-five Android instrumentation checks pass for real Keystore encryption,
GCM tamper rejection and access erasure; verified package/version/signature/ABI;
wrong package/certificate/downgrade rejection; provider access; ready-state
restoration; corrupted/partial download rejection; cancellation/retry; periodic
job network/battery/storage/reboot constraints and cancellation when disabled.
The signing-mismatch fixture has a valid different signature. Its temporary
signing key and password were removed after generation.

Real Android installer QA passes eleven checks: cold ready-state restoration,
library banner/navigation, automatic settings, install-source permission,
installer cancellation with byte-exact retained metadata, retry, actual APK
installation from0.27.1/code31 to a QA-only0.27.2/code32, preserved library rows,
cache cleanup, actual installed-version display, rotation and masked/invalid
access entry. The candidate was created through build-time version overrides,
not a production network override. QA APKs, fixtures and instrumentation do not
ship in the main app. An external URI read without a grant is denied.

The first installer cancellation test found a real cache-state bug: an updater
method named check shadowed Kotlin's assertion inside metadata reading and
started a release check. It was renamed checkForUpdates; the bound now explicitly
uses kotlin.check. A new device regression asserts that installer metadata reads
neither make a release request nor clear READY. The final device matrix passes.

The UI harness was corrected to use Android Settings' checkable Compose row,
to wait for the final Update confirmation instead of the moving Staging dialog,
and to open the completed installer before inspecting the library. A last
cache-cleanup assertion originally read adb exec-out's local exit status, which
stays0 even for a remote missing file; actual shell test-f confirms both the
cache cleanup and the existing app behavior. UI QA resumed from seven recorded
completed assertions, rechecked the installed version/library, then passed the
remaining four checks. Raw failures, XML, logs, screenshots and progress proofs
are retained under ignored reports/update-qa/v0.27.1-dev/ and reports/v0.27.1-*.
An unrelated System UI startup Wait and one unexpected graceful AVD shutdown
between attempts are recorded; the cause of that earlier shutdown is unconfirmed.

Final two-ABI assembly, instrumentation assembly, JVM tests, Android lint,
signature, strict component/permission verification,21 engine exports and16KiB
ELF/ZIP checks pass. The manifest has exactly INTERNET, ACCESS_NETWORK_STATE,
RECEIVE_BOOT_COMPLETED and REQUEST_INSTALL_PACKAGES. Only HomeActivity is an
exported activity; the read-only update provider is private and the exported
JobService requires the system BIND_JOB_SERVICE permission. The two native
libraries match0.27 exactly, and no QA APK/token fixture is in the main DEX/assets.
App/code/schema/workload/native:0.27.1-dev/31/4/30/15. APK12036168bytes, SHA256
`d379152c576613be5cbb799dc03d5c2f282d6f8d279de97343ece1efd33288ec`.

The periodic job configuration and controlled downloader are tested. A live user
credential connection is left to the phone's one-time setup; the test suite does
not grant the phone the build host's GitHub credentials. Android's actual
background cadence is controlled by the OS. The app explicitly asks for install
confirmation. The QA-only future APK is never published as a real release.

The final single-run regression passes all11 workflow checks with one fresh
240-check/55-pattern report, all7,040 displayed pixel centers, exact four Rust
group comparisons and all71 native records. Downloads export, sharing and cold
Diagnostics restoration are byte-exact. The automatic JSON stays immutable.
This mode deliberately omits unrelated interruption/restart repeats; the engine
native code is unchanged. Final UI/regression crash buffers are empty with no
target fatal exception, native signal or ANR. Synthetic access was erased, the
QA test package removed, display settings restored and the owned AVD stopped;
its process returned0 and ADB is empty. Raw earlier failures remain preserved.
The private GitHub API accepts the pinned2022-11-28 version from the authorized
host; actual per-phone access is still supplied by the user at setup.
