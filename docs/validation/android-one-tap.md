# One-tap workflow validation (0.22.1-dev)

The final APK passes Android assembly/lint, signature, arm64-v8a/x86_64 ELF,
all 18 Rust exports, 16KiB ELF/ZIP alignment and strict main-app manifest checks.
BatchActivity is nonexported; HomeActivity remains the only exported activity.
No storage/network permission or QA component was added. APK: 10,745,493 bytes,
SHA-256 `018e55f83ec24a70e346bbaeb84df5b599f82ddeeeb3bbd519a52a93f919d0fb`.
App/code/schema/workload/native are 0.22.1-dev/25/4/25/10. The Rust engine,
shader corpus and emulation behavior are unchanged from the 200-test/30-fixture
[0.22 baseline](android-indexed-batch.md); no new Rust capability is claimed.

## One-tap execution

`tools/qa_one_tap.py` passes 14 flows on the final APK:

- Home action launches the complete sequence; catalog/cache survive update.
- Cancel produces no new JSON.
- All six patterns advance automatically, each with real native pixel/completion/
  presentation evidence and a TextureView consumer update.
- Six display captures check all 128 source-pixel centers each (768 samples)
  against independent integer palette/coordinate/triangle formulas.
- All 134 checks pass: engine17/NV50/core20/shader25/Vulkan22. Every Rust group
  exactly matches the host baseline except architecture.
- Published Downloads JSON is byte-exact with the private cache, has MIME type
  application/json, correct relative path and IS_PENDING=0; no picker is used.
- Share opens the chooser and does not create another export.
- Completed rotation preserves the saved export.
- Rotation and backgrounding during an unfinished run restart it; abandoned run
  IDs/results cannot appear in the fresh report or publish an extra file.
- Each completed run produces one distinct JSON; old files remain intact.
- Cold Diagnostics restores all six pattern proofs and the complete report exactly.

Two complete automatic runs pass in the final QA, plus canceled/interrupted
attempts. The final same-version reinstall preserves all four catalog rows and
the current report. The earlier actual 0.22 to 0.22.1 update also preserved them;
no app-data clear was used. The harness initially accepted a pre-existing report
too early during a repeated same-workload run. It now excludes previous and
abandoned run IDs and requires a fresh result; the final run passes all 14 flows.
The manifest verifier was extended to require the new activity to be nonexported.

## Manual compatibility and recovery

All 14 existing manual scene checks pass on the final APK, including all six
patterns, rapid input, rotation, background/foreground and repeated teardown.
Ten captures check 128 displayed points each (1280 samples). The new shared JNI
scene bindings therefore work for both manual and automatic activities.

The standard export/restore runner resumes the newly executed automatic report;
it does not seed passing results or rerun an unrelated previous report. It
independently verifies all 134 records, real picker export, cold restore and
both present AtomicFile caches (diagnostics and texture snapshot), byte-exact.
The automatic report stays unchanged after later manual scene use, while the
separate texture cache still recovers from its backup. No current-version Catch
snapshot was created in this UI-focused run, so the third-cache recovery is not
claimed for this build; the unchanged Catch/ReportStore baseline is documented
in the preceding validation.

Final screenshots were visually inspected: automatic patterns/results,
portrait/landscape results, share chooser and cold Diagnostics. Completion uses
a scrollable results list instead of retaining/recreating the stopped GPU scene.
All three QA crash buffers are empty; logs contain no app fatal exception,
native fatal signal or app ANR. No System UI Wait was required.

The owned API35/x86_64 SwiftShader AVD started after 97.13% CPU idle, nice10,
two cores and 2048MiB requested (emulator raises to2560MiB). Boot completion was
verified. It was stopped after QA and its process awaited successfully. Raw
evidence remains ignored under `reports/android-qa/v0.22.1-dev/` and
`reports/v0.22.1-*`; final automatic evidence is in `one-tap-final-verified/`.

This is software-device workflow evidence. No new physical Mali report or ADB
session was supplied; the prior physical texture checkpoint remains documented
separately. Commercial games and general guest graphics/runtime support remain open.
