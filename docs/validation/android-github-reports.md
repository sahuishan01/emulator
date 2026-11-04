# Android GitHub report delivery — 0.27.4-dev

New app/code34, schema4/workload31/native16,252 diagnostics/55 patterns.
Both native libraries are byte-identical to0.27.3. No renderer fix, commercial
compatibility claim or new Android permission. Nine activities (only Home
exported), one private installer provider, two system-protected JobServices.

39 JVM tests pass:27 updater tests plus12 report-policy/network tests. New
coverage includes byte-exact UTF-8/whitespace identity, complete failing reports,
unsupported-group errors, size/schema/UTF-8/trailing-data rejection, actual
write acknowledgement, public repository rejection, denied/not-found/rate-limit/
redirect/server errors, response limits, issue/artifact creation, interrupted
upload recovery and exact remote readback rejection.

13 offline Python tests pass for report identity and failing evidence,
malformed/incomplete/nonfinite rejection, private/owner/PR guards, exact archive
permissions, duplicate collection, one-second cursor overlap, bounded backlog
continuation, incomplete/corrupt artifacts, busy-thread/user-queue deferral,
idle same-thread delivery once, lost enqueue acknowledgement recovery, consumed
entry handling, grouping new reports separately, workspace guard and data-only
prompt construction. These tests use isolated temporary directories and fake
GitHub/control transports; they do not start another model turn.

16 checks pass in the Android API35 owned emulator with a QA-only injected
network and synthetic Keystore credential: default off, denied enable, real
access-probe acknowledgement, manual upload while automation is off, exact-byte
dedup, verified enable, actual MediaStore Downloads publication and automatic
queue hook, immutable cache/export, offline queue and issue persistence,
reboot/network/battery/storage job constraints, manager recreation and retry,
global rate-limit waits, revoked-access suspension, explicit recovery,
cancellation preserving Downloads, visible actions/access history, disable,
disconnect clearing verification and immediate opt-out invalidating queued work.
The real exporter is called directly rather than rerunning unchanged252 native
checks. Main APK has no QA class, fake credential or endpoint override.

Live GitHub checks used the existing authenticated host account; the connector
was unavailable and authenticated `gh` was used. The current physical JSON was
uploaded/read back exactly (226569bytes/SHA256
86b35f40b56e027de637dfe2856e53d484761b323b31529950d7100089557255), including
its whitespace. A1048573-byte issue artifact was also read back exactly, then
deleted. The supplied report is now
[issue2/artifact1](https://github.com/sahuishan01/emulator/issues/2/artifacts/1).
No raw report is committed to source Git. Private artifact access on the phone
still requires the user's one-time Issues read/write permission and opt-in;
physical automatic upload is not yet evidenced by a new phone report.

The collector independently retrieved that artifact, checked identity/schema,
archived mode600, derived246/252 checks and54/55 patterns, and correlated all
four color mismatches with captured UV values. The live existing Codex socket
accepted thread/read and a temporary queue add/list/delete probe (removed before
execution). Actual active-thread collection defers correctly; repeated polls
do not redeliver the SHA. No separate agent/model process or daemon restart.
Idle-start behavior is tested against a controlled transport; the real pending
seed will be dispatched once this current turn becomes idle.

Systemd unit validation passes; installed timer is enabled/active with two-minute
schedule. The oneshot completes with Result=success/ExecMainStatus=0 and logs
knownReports1/waiting for idle thread. Limits25%CPU/256MiB/24tasks/180sec,
UMask0077. Config and state are private; no public listener is introduced.
GitHub credentials remain in existing protected host authentication and the
phone Keystore vault. The experimental local Codex control API is an ongoing
maintenance dependency; this is not an independently hosted OpenAI service.

Ignored evidence: `reports/v0.27.4-*`,
`reports/report-qa/v0.27.4-dev/`, `reports/github-inbox/`,
`reports/report-artifact-*`. Build logs preserve incremental attempts. One later
updater UI instrumentation run was interrupted by accidentally overlapping an
adb UI runner; its Process crashed instrumentation result is retained with an
empty crash buffer. That overlap was stopped and subsequent UI runs serialized.
No application crash is inferred from the harness force-stop.

The latest physical report remains246/252 with six related Vulkan failures;
all four Rust groups pass. All four captured inset color mismatches match the
colors predicted by the actual UV capture, which differs slightly from exact
boundaries. UniformW2/reversed winding/U offsets retain failures; unitW/V offsets
pass. Keep strict failure evidence. This feature does not change shader or
sampling behavior, bias UVs or relax comparison tolerances.

Final UI runs:14 updater UI/manager access cases pass on the final APK. Six adb
report workflows pass with fresh XML-derived taps: Settings discovery/default
opt-out, manual control availability, explicit disconnected access guidance,
denied-enable rollback, manage-connection navigation, landscape and cold restore.
Seven prior adb updater checks also pass, including actual signed33->34 update,
byte-exact library/cache preservation, masked/empty/cancel and manual actions.
The final report UI rerun was serialized after instrumentation and passed again.
Both final report screenshots and the verified-history screenshot were visually
inspected. Synthetic credentials/backups absent, test package uninstalled,
final crash buffer empty/no target fatal or ANR. The owned AVD was started at
CPU idle>=80%, nice10/2cores/2048MiB requested (emulator raised to2560), inherited
720x1280/density280, orientation restored, stopped and process awaited; ADB empty.

Final main/test assembly,39JVM tests, lint, signature, twoABIs,21 Rust exports,
16KiB ELF/ZIP alignment and strict manifest all pass. Existing nonfatal lint
warnings remain; no lint error. APK12497593bytes/SHA256
11195a81b01bc5077f0df556076cfb6e1f7425e29c24d4fc01b440d9f0f6ad23.
