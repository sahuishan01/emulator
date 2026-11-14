# Handoff

## Working Agreement & Invariants

- **Original Clean-Room Core**: Emulator written completely from scratch. Do not fork, link, copy, import, or vendor code from `upstream/` or third-party emulator cores. `upstream/` is purely historical reference and excluded from builds.
- **Target Hardware**: Realme GT 7 Dream Edition (`RMX5061` / `MT6989` / `Mali-G720-Immortalis MC12`). Eventual target game: Pokémon Legends: Z-A. Commercial games do not boot yet.
- **Execution Model**: Autonomous progress. Once a milestone or checkpoint is verified, proceed directly to the next planned task without repeatedly asking for user go-aheads.
- **Batch Verification**: Batch connected features, positive tests, rejections, rollbacks, and lifecycle coverage into single APK builds.
- **System Constraints**:
  - Maintain strict 16 KiB page-alignment across `arm64-v8a` and `x86_64` native libraries in APK builds.
  - Export all 22 required native engine symbols.
  - Respect CPU-gating (run heavy operations only when CPU idle $\ge 80\%$).
- **Notifications**:
  - `agent-releases`: Sent ONLY after successful build verification and publishing, containing permanent release-page and download links.
  - `agent-tasks`: Sent at the conclusion of tasks summarizing actions taken.

---

## Active Status & Latest Releases

- **Current Git HEAD**: `HEAD` on `main` (tracked at origin `sahuishan01/emulator`).
- **Latest Release**: `v0.28.7-dev` (versionCode 47).
- **Physical Device Telemetry (Issue #15)**:
  - Latest report: `1794318272960-44c3d786`, app version `0.28.6-dev` on `Mali-G720-Immortalis MC12`.
  - Diagnostics: 269/275 checks passed, 65/66 visible patterns passed.
  - Normalized-position candidate path passes all 23 checks (`normalized-position-texture-*` and `normalized-inset-*`) and all 11 candidate visible patterns (Variants 55–65) with 0 mismatched pixels.
  - Issues #5–#15 (0.27.6-dev through 0.28.6-dev) all have the same strict result. The baseline projected texture and five controls fail; candidate passes. Four baseline pixel mismatches match colors predicted by captured UVs. No new version-specific GPU regression is confirmed.

---

## Delivered Milestones

1. **Loader & File Formats**:
   - Clean-room PFS0 partition archive parser in [`crates/loader/src/pfs0.rs`](file:///root/projects/emulator/crates/loader/src/pfs0.rs).
   - NPDM metadata parser in [`crates/loader/src/npdm.rs`](file:///root/projects/emulator/crates/loader/src/npdm.rs) (`META` header, Title ID, product code, main thread priority, core affinity, stack sizing).
   - ExeFS program loader in [`crates/loader/src/exefs.rs`](file:///root/projects/emulator/crates/loader/src/exefs.rs) (sequential loading and mapping of `main.npdm`, `rtld.nso`, `main.nso`, `subsdk*.nso`).
   - Dynamic ELF relocation engine in [`crates/loader/src/reloc.rs`](file:///root/projects/emulator/crates/loader/src/reloc.rs) (`R_AARCH64_RELATIVE`).

2. **Horizon OS Services**:
   - `sm:` Service Manager registration and dispatch.
   - `set:sys`: `GetLanguageCode` (AmericanEnglish), `GetRegionCode` (USA) in [`crates/services/src/setsys.rs`](file:///root/projects/emulator/crates/services/src/setsys.rs).
   - `time:s` / `time:u`: Steady clock `GetCurrentTime` in [`crates/services/src/timesys.rs`](file:///root/projects/emulator/crates/services/src/timesys.rs).
   - `fsp-srv`: Filesystem proxy service (RomFS, SaveData mounts, `IFileSystem` proxy) in [`crates/services/src/fspsrv.rs`](file:///root/projects/emulator/crates/services/src/fspsrv.rs).
   - `vi:m`: Visual interface manager (`GetDisplayService`, `OpenDisplay`, `GetNaturalDisplayResolution` 1280x720, `CreateStrayLayer`) in [`crates/services/src/vi.rs`](file:///root/projects/emulator/crates/services/src/vi.rs).
   - `appletAE` / `appletOE`: Applet manager (`GetAppletResourceUserId`, `GetOperationMode`, `GetPerformanceMode`) in [`crates/services/src/applet.rs`](file:///root/projects/emulator/crates/services/src/applet.rs).
   - `hid`: Human Interface Device service (`CreateAppletResource`, `ActivateDebugPad`, `SetSupportedNpadStyleSet`, `SetSupportedNpadIdType`, `ActivateNpad`) in [`crates/services/src/hid.rs`](file:///root/projects/emulator/crates/services/src/hid.rs).

3. **AArch64 Execution Engine**:
   - PC-relative addressing (`ADR`, `ADRP`).
   - Logical shifted register operations (`AND`, `BIC`, `ORR`/`MOV`, `ORN`, `EOR`, `EON`, `ANDS`, `BICS`).
   - Bitfield extraction and sign extension (`UBFM`, `SBFM`).
   - Atomic exclusive load/store (`LDXR`, `STXR` 32-bit and 64-bit).

4. **NCA3 Metadata & Section Decryption Engine**:
   - Clean-room decrypted main-header parser in `crates/loader/src/nca.rs`.
   - Parses distribution/content fields, key-generation metadata, program/rights IDs, four indexed section entries, encrypted key area (`0x300..0x340`), and stored filesystem-header hashes.
   - Filesystem section header parser (`NcaFsHeader`) reading version, filesystem type, hash type, encryption type, and 8-byte section counter (`section_ctr`).
   - Verified section filesystem header SHA-256 integrity checks against main header stored hashes via `verify_fs_header_hashes()`.
   - Big-endian AES-CTR counter generation via `make_section_ctr` using the format-specific 8-byte reversed section counter prefix and media offset block index.
   - Section payload decryption via `decrypt_section()` supporting title-key content keys and key-area unwrap via AES-128-ECB. Sixteen NCA unit and integration tests pass. Details: [`docs/nca-parser.md`](docs/nca-parser.md).

5. **Crypto foundations**:
   - Clean-room dependency-free `emulator-crypto`: AES-128 block encrypt/decrypt, ECB decrypt, explicit-counter CTR, full-block XTS, FIPS 180-4 SHA-256 digest engine, and bounded `name = hex` key-file parsing with redacted debug output.
   - AES-128 FIPS 197, CTR SP 800-38A, XTS known-answer, and SHA-256 NIST test vectors pass. Lookup-table AES is not constant-time and is not production-hardened.

---

## Canonical Pokémon Legends: Z-A Execution Roadmap

The complete clean-room master plan for booting and running Pokémon Legends: Z-A on the Realme GT 7 Dream Edition (Arm Immortalis-G720) is documented in [`docs/pokemon-legends-za-plan.md`](file:///root/projects/emulator/docs/pokemon-legends-za-plan.md).

The implementation roadmap is structured into eight concrete milestones:
1. **Phase 1 (M5A) - NCA3 Decryption, Key Hierarchy & RomFS**: Section CTR decryption, Switch key derivation (`header_key`, `key_area_key`, `titlekeys`), RomFS IVFC level 4 directory/file stream reader, and multi-module ExeFS loader.
2. **Phase 2 (M5B) - AArch64 Floating Point & NEON Vectors**: Complete scalar FP arithmetic (`FADD`, `FSUB`, `FMUL`, `FDIV`, `FMADD`, `FCMP`, `FCVT`), 128-bit vector load/store (`LDR`/`STR` Q-registers, `LD1`–`LD4`), vector math/logic/permutations, and FPCR/FPSR status control.
3. **Phase 3 (M6A) - Horizon OS Multi-Core Kernel & Services**: 3-core guest scheduler simulation, user-space futex arbitration (`svcWaitProcessWideKeyAtomic`), full `fsp-srv`, `vi:m`/`nvnflinger` BufferQueue presentation, `hid` controller ring buffers, `audren:u` audio mixing, and `nvdrv` GPFIFO submission.
4. **Phase 4 (M6B) - Maxwell 3D GPFIFO Engine & State**: GPFIFO pushbuffer parsing (methods, gather buffers, call/return), full B197 3D state machine (16 VBs, 32 attrs, two-sided stencil, MRTs, independent blend), TIC/TSC samplers, and syncpoint semaphores.
5. **Phase 5 (M6C) - Maxwell SM53 Shader Translator & Compute**: Full SM53 opcodes, typed SSA IR, compute workgroup shared memory (`LDS`/`STS`), `BAR.SYNC`, perspective-correct fragment interpolation (`IPA`), and SPIR-V 1.3 emission.
6. **Phase 6 (M7A) - Host Vulkan Immortalis-G720 Renderer**: Compute-to-draw skinning barriers (preventing post-menu black screens), EDS capability-gated static pipeline fallback, GPU compute BCn decompression to ASTC/RGBA with retained GOB backing, and Reverse-Z depth clamping.
7. **Phase 7 (M7B) - Android ARM64 Native Code Execution (NCE)**: Host direct execution on Dimensity 9400e Cortex cores, 16 KiB-aligned virtual address space, and fast SVC signal interception (>60 MIPS/core).
8. **Phase 8 (M8) - Pokémon Legends: Z-A Acceptance Campaign**: Staged verification (Title Boot -> Menu -> Post-Menu Intro -> Lumiose City 3D Exploration -> Save/Load Durability at 30 FPS).

---

## Next Planned Steps (In Exact Priority Order)

1. **NCA crypto integration and key hierarchy (Phase 1 / M5A)**:
   - Confirm NCA's per-sector XTS tweak and section CTR counter encoding from format evidence before wiring the generic primitives to files.
   - Implement key derivation (`header_key`, `key_area_key`, titlekeys) and validate header/section metadata without logging or persisting user key material.
   - Add independent known-answer and invalid-key/malformed-container cases.

2. **AArch64 Floating Point & NEON Vector Operations (Phase 2 / M5B)**:
   - Implement scalar single/double precision FP arithmetic in `crates/a64/src/lib.rs` (`FADD`, `FSUB`, `FMUL`, `FDIV`, `FMADD`, `FCMP`, `FCVT`).
   - Implement 128-bit vector load/store instructions (`LDR`/`STR` Q-registers).

3. **Horizon VI BufferQueue & Presentation Pipeline (Phase 3 / M6A)**:
   - Implement clean-room `nvnflinger` BufferQueue state machine (`Connect`, `RequestBuffer`, `QueueBuffer`, `DequeueBuffer`).
   - Connect buffer presentation to the Android Vulkan swapchain / `ANativeWindow`.

4. **NV Channel GPFIFO Submission Pipeline (Phase 4 / M6B)**:
   - Support `NVIOC_CHANNEL_SUBMIT_GPFIFO` command buffer submission.
   - Wire guest command stream decoding directly into the host Vulkan command processor.
