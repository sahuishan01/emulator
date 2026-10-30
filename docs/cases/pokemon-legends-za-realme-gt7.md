# Pokémon Legends: Z-A / Realme GT 7 Dream Edition

Historical comparison observations. The current project is an original
emulator; the earlier requests to change Lemon/Eden settings are superseded.
Our implementation milestones are in [../architecture.md](../architecture.md).

## Reported behavior

User report on 2026-10-30:

| Item | Evidence |
| --- | --- |
| Device | Realme GT 7 Dream Edition, supplied by user |
| Game | Pokémon Legends: Z-A, supplied by user |
| Lemon | Black screen after main menu, supplied by user |
| Lemon Lite | Black screen after main menu, supplied by user |
| Eden | Crashes, supplied by user; stage not yet specified |
| Model SoC / GPU | Dimensity 9400e / Arm Immortalis-G720, Realme specifications |
| Model memory | 16 GB RAM / 512 GB storage, Realme Dream Edition specifications |
| Android / driver / app versions / game build | Unknown; no live report supplied |
| Audio / FPS after black screen | Unknown |

Model specifications do not establish the OS or driver actually installed.
The pinned source checkouts are inspection baselines, not necessarily the
versions installed on the user's phone. None of the proposed tests below has
been run on this device yet.

## Source findings and limits

- In the pinned Lemon source, Android defaults to Extended Dynamic State
  disabled and Vertex Input Dynamic State disabled (`src/common/settings.h`).
  User imports, per-game overrides and different releases can change this.
- `Device::Device` / its suitability logic applies the selected EDS level in
  `src/video_core/vulkan_common/vulkan_device.cpp`. EDS2 retains supported
  EDS1/EDS2 features while removing EDS3. Unsupported extensions must remain
  disabled; a setting cannot manufacture driver support.
- `PipelineCache` consumes those choices in
  `src/video_core/renderer_vulkan/vk_pipeline_cache.cpp`.
- The existing BCn-to-ASTC/EAC paths in Lemon mean native BCn absence is not
  sufficient to diagnose this black screen. Copies/aliases, compute shader
  behavior, memory pressure and CPU/GPU visibility remain candidates.
- Eden issue #481 reports Z-A corruption after leaving a station, fixed for
  two Linux AMD users by EDS2 with Vertex Input Dynamic State disabled. A
  Windows AMD commenter reports that this did not fix their case. Neither
  result establishes a Mali fix or matches this phone's exact failure.
- Eden issue #545 reports Z-A going black with continuing music/FPS on a
  Samsung S24 FE when exiting a reward area. This is a different device and
  scene; use it only to distinguish rendering failure from a stopped process.

## Earlier diagnostic proposal (superseded)

First export an existing failing-run log and this game's configuration. Record
the app's exact version, game version, Android build, system driver, any custom
driver and whether audio/FPS continue. Do not replace saves or game content.

Use one Lemon build for the initial matrix. Select the system driver, record
the per-game settings, and hold resolution, CPU mode, GPU mode and shader
settings constant for each paired comparison. Start each run from a fresh
emulator launch and reproduce the same menu-to-scene transition.

| Pair | Single variable | What the comparison can tell us |
| --- | --- | --- |
| First | EDS Disabled vs EDS2; keep Vertex Input Dynamic State off | Whether the static/EDS pipeline path changes this failure on the actual driver |
| Follow-up | Recompress BCn textures on vs off; restore all other baseline settings | Whether compressed-host texture conversion/copy handling changes the failure; record the increased memory cost when off |
| Follow-up | NCE vs Dynarmic JIT; restore the graphics baseline | Whether the CPU execution/memory visibility path changes the failure; low JIT speed is not a graphics failure |
| Follow-up | Identical settings on default and compatible alternate driver | Whether a driver-dependent difference exists; test only a driver verified for the exact GPU/kernel |

Capture a result, screenshot, audio/FPS state, log and run duration for every
row tested. If the settings are not available in the installed version, record
that rather than inventing a matching configuration. Restore a changed setting
after a failed comparison.

An EDS result would narrow the investigation; it would not by itself identify
the faulty state. Reduce a changed result to a draw/pipeline test before adding
a permanent device/game workaround. If all runs fail, use the exported logs
to choose the next test rather than applying every workaround simultaneously.

## Evidence collection

Lemon's inspected Android source provides Settings → Help → Share logs, with
separate debug and GPU log exports (`SettingsActions.showShareLogsDialog`).
Menu placement may vary by installed version. Enable "Flush debug logs by
line" for a failing-run capture so the end of the log is less likely to remain
buffered. Export promptly before repeated launches rotate the logs.

For ADB, run the host collector with the actual package name. It records
Android exit history even if Eden has already crashed; match exit timestamps
to the reproduced failure. No local physical-device run has been performed.

## Sources

- Realme Dream Edition: https://www.realme.com/in/realme-gt-7-x-astonmartinf1-2025
- Realme GPU specifications: https://www.realme.com/sg/realme-gt-7-5g/specs
- Z-A Vulkan state report: https://github.com/eden-emulator/Issue-Reports/issues/481
- Z-A black-screen report: https://github.com/eden-emulator/Issue-Reports/issues/545
- Lemon setting descriptions: https://lemon-emu.org/guide.html
- Historical reference revisions: ../research/references.lock.json
