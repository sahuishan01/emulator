# Private in-app updates (0.27.1)

Install this APK once to add the updater. Open **Settings → App updates → Connect
GitHub**. Create a fine-grained token restricted to **sahuishan01/emulator** with
**Contents: read-only**, then enter it on the phone. The source repository and
release downloads stay private. No shared GitHub credential is bundled into the
APK, copied from the build host, or sent in diagnostic JSON.

Automatic checks are enabled by default. They run when the library opens and
through a persisted six-hour JobScheduler job, subject to Android scheduling,
network, battery and storage constraints. Checks are throttled; the manual
button can check immediately. Verified downloads use unmetered networks when
**Download on Wi-Fi** is enabled. The download button also works manually.
Progress, cancellation, retry and a ready-to-install action survive navigation;
completed downloads are verified again after process restart. Partial downloads
are removed and retried from the beginning. Android always asks the user before
installation, including its one-time **Allow from this source** permission.
Installation is never silently started by the periodic job.

The current private development channel includes published prereleases. The
client reads the latest 30 GitHub releases, ignores drafts and compares strict
semantic versions rather than API order or the latest-release endpoint. It
requires one uploaded emulator.apk with a bounded size and SHA-256 digest.
Downloads and metadata have size/time bounds, TLS and exact host/path rules.
Authorization is sent only to the repository's GitHub API routes and is removed
before following an approved asset-CDN redirect. Redirected URLs and raw network
exceptions are not exposed to the UI or persisted as credentials.

A token is saved only after a successful release query. Android Keystore holds
a non-exportable AES key; AES-GCM encrypts the token in no-backup storage, with
fresh IVs and authenticated context. Replacing access requires another successful
query. Disconnecting cancels transfer work, removes access and cached APKs, and
cancels the periodic job. Backup is disabled for this development app.

Before offering installation, the app checks all downloaded bytes, the expected
package and version name, a strictly higher versionCode, the installed signing
certificate, Android minimum version and a supported native ABI. Cached APKs
are checked again before the installer opens. A private read-only provider
exposes only the current checksum-named URI through a temporary read grant;
write access, traversal, stale hashes and external access without a grant fail.
The installer still performs Android's package verification.

The new manifest permissions are INTERNET, ACCESS_NETWORK_STATE,
RECEIVE_BOOT_COMPLETED (persisted jobs) and REQUEST_INSTALL_PACKAGES. The only
exported activity remains the library. The updater's JobService requires the
system BIND_JOB_SERVICE permission. No test endpoint override, QA APK or test
credential ships in the application.

For future releases, increment versionCode, use a matching semantic version
name/tag, retain the signing identity and publish a verified emulator.apk asset.
GitHub supplies its SHA-256 digest. Existing release publication remains private;
no update backend, public APK feed or website deployment is introduced.

The 0.27.1 updater release left the emulator/native diagnostic workload unchanged:
240 checks and 55 patterns. The 0.27.2 investigation build retains the updater and
adds 12 native controls, for 252 checks and 55 patterns.
The supplied [0.27 Mali report](validation/g720-projection.md) records one
projected inset-texture mismatch (239/240 and 54/55). This update does not fix or
hide that rendering issue. See [updater validation](validation/android-app-updates.md)
for separate JVM, device, installer and regression evidence.
