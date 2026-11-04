# Private GitHub reports and automatic follow-up

App **0.27.4-dev/code34** adds **Settings → GitHub reports**. Add **Issues:
read and write** to the phone's fine-grained token, restricted to
**sahuishan01/emulator**. Retain **Contents: read-only** for APK updates.
Deployments access is unnecessary. Replace the token in **App updates** if
required, then use **Test report upload access**. This verifies private repository
identity and a real write to the report inbox; release-read access alone does
not establish report-write access. Enable **Upload completed reports automatically**.
This setting is off until explicitly enabled after successful verification.

Run **Test all & export JSON** normally. A complete report, including failing
checks, is published in Downloads first and queued for private GitHub delivery.
**Upload latest JSON** also works manually with automatic uploads off.
The latest cached report is queued when automatic uploads are enabled.
Background work uses Android JobScheduler and needs network, sufficient battery
and storage. Android controls scheduling; force-stop suspends work until the app
is reopened. Offline errors retain pending data. Rate limits delay manual and
background requests, permission failures suspend background retries until access
is verified again. Disconnect stops uploads and clears access verification.

Each report has a SHA-256 identity and its own private issue linked to
[the inbox](https://github.com/sahuishan01/emulator/issues/1). Its issue artifact
contains the **original UTF-8 JSON**, including whitespace. Before acknowledging
an upload the app reads the artifact back and checks exact bytes and digest.
Saved issue IDs and artifact lookup recover interrupted uploads. The queue
retains up to32 reports of at most1MiB each, plus64 recent receipts. Queue-full
errors preserve Downloads. This bounded history does not promise perpetual
deduplication of ancient manually resubmitted reports.

The upload includes device/driver data, test results, comparison evidence and
report export metadata. It does not upload games, firmware, keys or credentials.
The shared token remains encrypted with Android Keystore; no host token is
bundled into the APK. Requests go only to the fixed private repository API,
without redirects, and refuse a public destination. Reports stay outside Git
source history. Local exports remain available independently of networking.
**Cancel pending uploads** removes queued copies; it keeps local Downloads and
already published GitHub reports. Disabling automation keeps pending data.

The API uses GitHub issue artifacts with API version2026-03-10. The phone posts
one generic artifact containing the raw JSON, then reads it back. A226569-byte
physical report and a1048573-byte size-boundary artifact were written and read
back exactly against the live private repository; the synthetic boundary artifact
was then deleted. See [validation](validation/android-github-reports.md).

## Server collection and the existing Codex thread

`tools/report_inbox.py` provides bounded publish/collect/analyze commands using
the existing authenticated `gh`. `tools/report_watch.py` runs every two minutes
through `emulator-report-watch.timer` on this VPS. It checks private visibility,
owner-authored issue/artifact identity, size, schema and SHA; saves raw JSON and
derived analysis under ignored `reports/github-inbox/`; and deduplicates delivery
by SHA. Incomplete uploads are retried; invalid artifacts never become prompts.
The initial issue2 contains the supplied0.27.3 physical report,246/252 checks and
54/55 patterns. All four captured color errors agree with the captured UV values;
this is evidence for the next rendering investigation, not a confirmed fix.

New reports queue a fixed trusted request into the **same existing Codex thread**
through its private local Unix WebSocket. Raw report text is data, never prompt
instructions. The collector waits while the thread is active and respects queued
user work. It preserves the thread's existing model/settings, starts no separate
agent and does not restart the managed daemon or expose a network listener.
Stored queue IDs and a persisted handoff identity recover lost acknowledgements;
consumed or deliberately removed entries are not resubmitted. At most16 new
reports are grouped into one handoff. Existing report hashes do not trigger
repeated model polling.

The private config is `~/.config/emulator-reports/watch.json`; collection and
delivery state is `~/.local/state/emulator-report-watch/state.json` (mode600).
The timer is enabled across boot, with25% CPU,256MiB RAM and24 tasks maximum
for the collector. Collection requires this VPS, authenticated GitHub access and
the existing Codex daemon/thread to remain available. The local control API is
experimental and may need maintenance after Codex upgrades. If a new physical
run is required, the resumed work ends at that checkpoint; it cannot operate
the phone or silently install a new APK. Android still asks before installation.

Inspect with `systemctl status emulator-report-watch.timer` and
`journalctl -u emulator-report-watch.service`. Stop automatic collection/resumption
with `systemctl disable --now emulator-report-watch.timer`, then stop a currently
running `emulator-report-watch.service` if needed. This leaves uploaded reports
and source work intact. The project copies of both units are in `tools/systemd/`.

No rendering/emulation code changes in this version. Diagnostics remain
schema4/workload31/native16,252 checks and55 patterns. Both native libraries are
byte-identical to0.27.3. The Mali inset failure remains open and commercial
Switch games, including Pokémon Legends: Z-A, remain unsupported.
