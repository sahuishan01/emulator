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

- **Current Git HEAD**: `c003c1f` on `main` (tracked at origin `sahuishan01/emulator`).
- **Latest Release**: `v0.28.6-dev` (versionCode 46).
- **Physical Device Telemetry (Issue #14)**:
  - Report: `1794317303643-da92c9f4`, app version `0.28.5-dev` on `Mali-G720-Immortalis MC12`.
  - Diagnostics: 269/275 checks passed, 65/66 visible patterns passed.
  - Normalized-position candidate path passes all 23 checks (`normalized-position-texture-*` and `normalized-inset-*`) and all 11 candidate visible patterns (Variants 55–65) with 0 mismatched pixels.

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

---

## Next Planned Steps (In Exact Priority Order)

1. **NCA (Nintendo Content Archive) Container Parser**:
   - Implement clean-room `NCA3` container parser in `crates/loader/src/nca.rs`.
   - Parse NCA header structures: magic (`NCA3`), distribution type, content type (Program, Meta, Control, Manual, Data), key generation, section entries (up to 4 sections), and section hash tables.
   - Add test coverage verifying headers, section table bounds, and malformed header rejection.

2. **Clean-room Cryptography & Key Management**:
   - Implement AES-128-CTR and AES-128-XTS block operations for header and section decryption.
   - Implement keyset loader (`prod.keys`) for titlekeys, header keys, and key area encryption keys.

3. **AArch64 Floating Point & NEON Vector Operations**:
   - Implement scalar single/double precision FP arithmetic in `crates/a64/src/lib.rs` (`FADD`, `FSUB`, `FMUL`, `FDIV`, `FCMP`, `FCVT`).
   - Implement 128-bit vector load/store instructions (`LDR`/`STR` Q-registers).

4. **Horizon VI BufferQueue & Presentation Pipeline**:
   - Implement clean-room `nvnflinger` BufferQueue state machine (`Connect`, `RequestBuffer`, `QueueBuffer`, `DequeueBuffer`).
   - Connect buffer presentation to the Android Vulkan swapchain / `ANativeWindow`.

5. **NV Channel GPFIFO Submission Pipeline**:
   - Support `NVIOC_CHANNEL_SUBMIT_GPFIFO` command buffer submission.
   - Wire guest command stream decoding directly into the host Vulkan command processor.
