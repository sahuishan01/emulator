# Mali rendering investigation

Historical notes from the earlier fork/patch approach. The user subsequently
requested an emulator written from scratch. The active implementation and
milestones are described in [architecture.md](architecture.md); these findings
do not constitute our emulator implementation or a plan to modify another app.

## Target and acceptance

The current target is Pokémon Legends: Z-A on the Realme GT 7 Dream Edition
(Dimensity 9400e / Immortalis-G720 according to Realme specifications). The
user reports a black screen after the main menu in Lemon and Lemon Lite, and
a crash in Eden. Exact installed versions, settings, game version, driver and
logs remain unknown. The reproduction and test matrix are in
[the case record](cases/pokemon-legends-za-realme-gt7.md).

The first success criterion is a named failing scene rendering correctly on
the same physical phone, with a recorded emulator commit, game/build version,
driver, settings, screenshot and run duration. Establish rendering correctness
before setting a performance target based on the game and device.

## Verified source observations

Reference revisions are recorded in research/references.lock.json, checked on
2026-10-30. They are not dependencies of our original Rust implementation.

- Eden marks unsupported BCn textures for conversion in
  `src/video_core/renderer_vulkan/vk_texture_cache.cpp`, `Image::Image`.
  Therefore missing native BCn sampling does not by itself demonstrate that
  all games using those textures must fail.
- Lemon's `TextureCacheRuntime` configures BCn-to-ASTC and BC4/BC5-to-EAC
  recompression based on reported capabilities and settings. Its
  `vk_compute_pass.cpp` contains corresponding compute passes, with shader
  sources under `src/video_core/host_shaders/`.
- Lemon also maintains raw-block data for copies involving converted BCn
  images. A conversion-only patch could break copies, aliases or readbacks
  even if a simple sampled texture looks correct.
- Eden has existing ARM driver handling and a driver-bug document. Existing
  workarounds must be inspected before adding a broad vendor-specific switch.
- Lemon's website and recent release notes describe Mali support, while its
  README still contains an older Adreno-only scope paragraph. Treat upstream
  support statements as leads; verify the source and exact device/game.

These observations identify code to investigate. They do not establish the
cause of the user's failure, nor do they validate upstream gameplay claims.

## Working hypotheses

| Area | Evidence to collect | Candidate action after reproduction |
| --- | --- | --- |
| Texture formats | Per-format sampled/storage/transfer support; bad texture or failed allocation | Correct fallback or GPU conversion with matching channel, signedness, sRGB and HDR semantics |
| Texture copies/aliasing | Minimal raw-block copy, mip/array/3D workload | Preserve guest data and correct block/texel extents across representations |
| Shader translation | Failed SPIR-V/pipeline, missing feature, reduced shader | Lower the specific operation or add an applicable capability fallback |
| Synchronization | Deterministic compute-to-draw/transfer workload; validation output | Correct barriers and resource lifetime; avoid blanket queue-idle waits |
| Memory pressure | Process memory, texture allocations, Android exit reason | Budget conversion/staging memory and bound caches without evicting live resources |
| CPU/GPU interaction | Controlled JIT/NCE comparison, coherent-memory/readback evidence | Correct visibility/invalidation at the responsible boundary |

Do not assume a missing geometry or tessellation feature caused a failure
unless the game actually uses it. Do not treat advertised Vulkan version or
extension names alone as proof that a rendering workload works.

## Reproduction record

Capture one record per test:

- Phone/model, SoC, GPU, RAM, Android build, system driver and loaded custom
  driver (if any).
- Emulator source commit, package version, game version/build ID, firmware
  version, enabled updates, mods and graphics/CPU settings.
- Exact save/scene and steps; distinguish crash, black scene with visible UI,
  corrupt geometry/textures, hang, and low frame rate.
- Screenshot, exported emulator log, device report and whether failure is
  repeatable from a cold start.
- One known-working control scene/game on the same setup.

Keep game data, keys, firmware, captures and logs outside version control.

## Validation sequence

1. Baseline: stock system driver, recorded settings and an unchanged source
   revision. Determine whether a current upstream build already fixes it.
2. Change one relevant variable at a time. For example, compare JIT and NCE
   if black rendering could stem from CPU/GPU visibility. Disable mods and
   generated frames during correctness comparisons.
3. Reduce the failure where possible to a redistributable homebrew or Vulkan
   workload. Use Vulkan validation and shader tools in test builds.
4. Compare the targeted patch against baseline on the exact same scene and
   on a working control; include cold and warm shader-cache runs.
5. Test sustained play, scene changes and suspend/resume. Record frame times,
   memory and temperature when setting a device-specific performance target.
6. Repeat on other physical Mali generations and a non-Mali control before
   expanding the support claim.

Standard upstream PanVK support does not imply an installable driver for a
stock Android phone's vendor kernel. Investigate a custom driver only for a
specific compatible GPU/kernel/device combination. Start with the system
driver so emulator and driver changes can be evaluated separately.

## Sources

- Eden source: https://git.eden-emu.dev/eden-emu/eden
- Lemon source: https://git.lemon-emu.org/lemon/Lemon-Project
- Lemon Mali notes: https://lemon-emu.org/faq.html
- Mesa Mali driver support: https://docs.mesa3d.org/drivers/panfrost.html
- Arm's device-report example: https://learn.arm.com/learning-paths/mobile-graphics-and-gaming/streamline-nx/prepare-tools-and-device/
