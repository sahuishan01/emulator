# Held-direction controls

Patch0.19.1-dev/code20/reportworkload20 keeps the103 cases and native GPU
workload7 unchanged. Final APK SHA256:
`2be150f7dc6ceddf64f370b855a92c29132e05a18083a099a7b14b780d05539b`
(9,683,766bytes). The ARM64 and x86_64 native libraries are byte-identical to the
verified0.19.0 APK; no Rust, native rendering or guest program changed.

Left/Right used only OnClick, so a held finger produced a single step. The patch
moves immediately on DOWN, repeats after240ms at80ms intervals, and cancels on
UP/CANCEL, active pointer lift or leaving the button. Accessibility clicks and
taps keep one step (UP does not add another). Opposite held directions neutralize.
Pause/drag/window-focus loss/background/surface teardown/destruction clear both
held state and callbacks. The UI-thread scheduler owns button state; renderer
ownership and the bounded input mailbox remain unchanged.

Pinned two-ABI build and lint pass. Signature, both ELF machines, all ten engine
exports and16KiB ELF/ZIP alignment verify. This is packaging evidence, not a
16KiB-page runtime test. On the owned API35/x86_64 SwiftShader AVD,
`qa_homebrew.py --hold-controls` passes all17 recorded flows: left/right repeat,
release stopping before the edge, both clamps, moving outside cancelling,
background while held and no resumed hold, followed by the existing visible
pixel, pause/tap/drag/reset/D-pad/score and repeated teardown checks. Actual
DOWN/UP events maintain the hold; assertions allow the500ms report capture to
settle before comparing later frames. Red/green/black screen pixels and complete
readback oracles are checked independently. Final screenshot visually inspected;
no app fatal exception or crash buffer entry captured. Artifacts are ignored
under reports/android-qa/v0.19.1-dev/homebrew-complete/.

Earlier harness attempts are retained: a stationary swipe produced only a single
step in this AVD test; explicit DOWN/UP injection exercises held state. The first
release assertion compared a cached during-hold capture with a later final
position; it now settles the capture cadence and compares two later advancing
frames. These failed harness attempts are not passing evidence.

This AVD started after97.56% idle with nice10/two cores/2048MiB requested; the
emulator raised RAM to2560MiB. Boot was awaited (73seconds), no snapshot/headless
SwiftShader. The physical0.19.0 report separately passes all103 and records578
hardware G720 frames with no demo error; see [the checkpoint](g720-live-homebrew.md).
No new commercial compatibility or performance claim follows from this UI fix.

The final fresh103-case run passes exact four-group reference comparisons,
workload19 invalidation, visible summary and byte-exact export/cold restore.
Both final batch screenshots were visually inspected; its crash buffer is empty
and the runner finds no app fatal signal/exception/ANR. See batch-retry/verification.json.
An earlier batch attempt was inconclusive after ADB returned255 and log captures
timed out. After restart, run-as initially reported the package unknown. The
same verified APK was reinstalled and the authentic before-upgrade workload19
cache was restored byte-exactly as a cache-invalidation fixture; the workload20
results were generated afresh by the app. Both attempts and the fixture-restoration
record are retained. The restarted AVD passed its98.28% idle gate, boot was
awaited (80seconds), and the owned process was stopped/awaited (exit0). No adb
devices remain. No failed attempt is presented as successful evidence.
