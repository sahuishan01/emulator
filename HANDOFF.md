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

4. **NCA3 Metadata Parser**:
   - Clean-room decrypted main-header parser in `crates/loader/src/nca.rs`.
   - Parses distribution/content fields, key-generation metadata, program/rights IDs, four indexed section entries, and stored filesystem-header hashes.
   - Validates alignment, archive-size agreement, reserved fields, section bounds and overlap. Ten NCA tests plus the existing loader suite pass.
   - NCA3 header-only XTS decryption is now wired to the `header_key` lookup. It does not decrypt section payloads, verify signatures or section hashes, parse section filesystems, or derive keys. Details: [`docs/nca-parser.md`](docs/nca-parser.md).

5. **Crypto foundations (in progress)**:
   - Added dependency-free `emulator-crypto`: AES-128 block encrypt/decrypt, explicit-counter CTR, full-block XTS, and bounded `name = hex` key-file parsing with redacted debug output.
   - AES-128 FIPS 197, CTR SP 800-38A and XTS known-answer tests pass. Lookup-table AES is not constant-time and is not production-hardened.
   - NCA header sectors use the format-specific big-endian sector-index tweak. NCA section CTR counter construction, key hierarchy/derivation and authenticated signature/hash checks are still open. Do not claim full NCA/content decryption works.

---

## Next Planned Steps (In Exact Priority Order)

1. **NCA crypto integration and key hierarchy**:
   - Confirm NCA's per-sector XTS tweak and section CTR counter encoding from format evidence before wiring the generic primitives to files.
   - Implement key derivation and validate header/section metadata without logging or persisting user key material.
   - Add independent known-answer and invalid-key/malformed-container cases.

2. **AArch64 Floating Point & NEON Vector Operations**:
   - Implement scalar single/double precision FP arithmetic in `crates/a64/src/lib.rs` (`FADD`, `FSUB`, `FMUL`, `FDIV`, `FCMP`, `FCVT`).
   - Implement 128-bit vector load/store instructions (`LDR`/`STR` Q-registers).

3. **Horizon VI BufferQueue & Presentation Pipeline**:
   - Implement clean-room `nvnflinger` BufferQueue state machine (`Connect`, `RequestBuffer`, `QueueBuffer`, `DequeueBuffer`).
   - Connect buffer presentation to the Android Vulkan swapchain / `ANativeWindow`.

4. **NV Channel GPFIFO Submission Pipeline**:
   - Support `NVIOC_CHANNEL_SUBMIT_GPFIFO` command buffer submission.
   - Wire guest command stream decoding directly into the host Vulkan command processor.
