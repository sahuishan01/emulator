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
- **Latest Release**: `v0.28.21-dev` (versionCode 61).
  - Resolved `unsupported A64 instruction 0x1a9f17e8 at 0x100338` encountered during retail NSO execution (Pokémon Legends: Z-A at instruction #944).
  - Corrected conditional select decoding mask from `0x7fe0_0c00 == 0x1a80_0000` to `0x3fe0_0800 == 0x1a80_0000`, enabling full support for `CSINC`, `CSINV`, `CSNEG` and compiler aliases `CSET`, `CSETM`, `CINC`, `CINV`, `CNEG`.
  - Implemented A64 add/subtract with extended register `ADD`, `ADDS`, `SUB`, `SUBS` (`0x0b20_0000`) supporting all extend types (`UXTB`, `UXTH`, `UXTW`, `UXTX`, `SXTB`, `SXTH`, `SXTW`, `SXTX`), immediate shifts 0..=4, SP destination/source handling, and compiler aliases `CMP` / `CMN` with extended register.
  - Expanded load/store exclusive instructions to all operand sizes (8-bit, 16-bit, 32-bit, 64-bit) and added full acquire/release exclusive support (`LDAXR`, `STLXR`).
  - Implemented load-acquire and store-release non-exclusive memory operations `LDAR`, `STLR`, `LDARB`, `STLRB`, `LDARH`, `STLRH` (`0x0880_fc00`).
  - Implemented signed load pair instruction `LDPSW` (`opc == 1`) with 32-bit to 64-bit sign extension and corrected signed offset calculation in `LoadStorePair` and `FpLoadStorePair`.
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all 30 A64 unit tests.
- **Previous Release**: `v0.28.20-dev` (versionCode 60).
  - Resolved `unsupported A64 instruction 0xdac001ae at 0x1017d0` encountered during retail NSO execution (Pokémon Legends: Z-A at instruction #362).
  - Implemented A64 Data-processing (1 source) instructions: `RBIT` (reverse bits, 32-bit and 64-bit), `REV16` (reverse bytes in 16-bit halfwords, 32-bit and 64-bit), `REV` / `REV32` (reverse bytes in 32-bit words), `REV` (reverse bytes in 64-bit doublewords), `CLZ` (count leading zeros, 32-bit and 64-bit), and `CLS` (count leading sign bits, 32-bit and 64-bit).
  - Implemented ARMv8 architectural `HINT` instructions (including `NOP`, `YIELD`, `WFE`, `WFI`, `SEV`, `SEVL`, `BTI`, `PACIASP`, `AUTIASP`, `PACIAZ`, `AUTIAZ`, `PACIBSP`, `AUTIBSP`, `PACIBZ`, `AUTIBZ`, `PACIA1716`, `AUTIA1716`, `PACIB1716`, `AUTIB1716`, `XPACLRI`), memory barriers (`DMB`, `DSB`, `ISB`), and exclusive monitor clearing (`CLREX`) under mask `0xffff_e01f == 0xd503_201f`.
  - Expanded bitfield operations to support `BFM` (`opc == 1`), enabling `BFI` (Bitfield Insert) and `BFXIL` (Bitfield Extract and Insert Low) with destination register bit preservation.
  - Implemented register extraction instruction `EXTR` (`0x1380_0000` / `0x9380_0000`) for 32-bit and 64-bit concatenated extraction and rotation.
  - Implemented add/subtract with carry instructions `ADC`, `ADCS`, `SBC`, `SBCS` (`0x1a00_0000`) with full NZCV carry-in and borrow flag evaluation.
  - Implemented conditional compare instructions `CCMP` and `CCMN` (both register and immediate variants) under mask `0x3fe0_0410 == 0x3a40_0000` with condition evaluation and default immediate NZCV setting.
  - Expanded 3-source multiplications to support signed and unsigned wide multiplications `SMADDL`, `SMSUBL` (including `SMULL`, `SMNEGL`), `UMADDL`, `UMSUBL` (including `UMULL`, `UMNEGL`), and high-word multiplications `SMULH`, `UMULH`.
  - Maintained strict 16 KiB page alignment across `arm64-v8a` and `x86_64` architectures, verified all 22 native engine exports, and passed all 26 A64 unit tests.
- **Previous Release**: `v0.28.19-dev` (versionCode 59).
  - Resolved `unsupported A64 instruction 0xd63f0020 at 0x1000bc` encountered during retail NSO execution (Pokémon Legends: Z-A).
  - Implemented A64 unconditional register branch instructions: `BLR Xn` (`0xd63f0000 | (rn << 5)`), `BR Xn` (`0xd61f0000 | (rn << 5)`), and `RET Xn` (`0xd65f0000 | (rn << 5)`).
  - Implemented test bit and branch instructions `TBZ` / `TBNZ` (`0x3600_0000` / `0x3700_0000`) for conditional branching based on tested bit values.
  - Implemented conditional select instructions `CSEL`, `CSINC`, `CSINV`, `CSNEG` (unlocking compiler aliases `CSET`, `CSETM`, `CINC`, `CINV`, `CNEG`).
  - Implemented integer multiply instructions `MADD`, `MSUB` (including `MUL`, `MNEG`), integer divide `UDIV`, `SDIV`, and register variable shifts `LSLV`, `LSRV`, `ASRV`, `RORV`.
  - Implemented logical immediate instructions with bitmask decoding `AND`, `ORR`, `EOR`, `ANDS`.
  - Expanded memory load/store operations to support 8-bit (`STRB`, `LDRB`, `LDRSB`), 16-bit (`STRH`, `LDRH`, `LDRSH`), 32-bit (`STR`, `LDR`, `LDRSW`), 64-bit (`STR`, `LDR`), unscaled `LDUR` / `STUR`, pre/post-indexed addressing, and register offset addressing `[Xn, Xm]`.
  - Corrected retail NSO startup registers in `Session::from_exefs_with_services` to initialize `x0 = 0`, `x1 = MAIN_THREAD_HANDLE` (0x100), and write `MAIN_THREAD_HANDLE` to `TLS_PAGE + 0x110` according to Nintendo Horizon OS process launch conventions.
- **Previous Release**: `v0.28.18-dev` (versionCode 58).
  - Resolved `invalid loader input: failed to parse NSO module 'rtld': invalid loader input: NSO section SHA-256 hash mismatch` when loading commercial Switch NSP packages (including Pokémon Legends: Z-A).
  - Corrected NSO0 section SHA-256 hash offsets in the header from `0x70 + i * 0x20` to `0xA0 + i * 0x20` (Text hash at 0xA0..0xC0, RoData hash at 0xC0..0xE0, Data hash at 0xE0..0x100) per Nintendo Horizon NSO0 header specifications.
  - Properly authenticated decompressed section digests against header digests when hash checking flags (bits 3, 4, 5) are active.
  - Added unit test coverage validating SHA-256 hash calculation and verification at offset 0xA0, as well as rejection of corrupted section payloads.
- **Previous Release**: `v0.28.17-dev` (versionCode 57).
  - Resolved `invalid loader input: ExeFS contains no valid NSO modules` on commercial Switch NSP game packages.
  - Fixed NSO0 header flags offset to `0xC` (per Horizon NSO0 format specification; retaining `0x8` for backwards compatibility) ensuring compressed retail NSO sections are properly recognized and decompressed with LZ4.
  - Removed strict page-multiple restriction on NSO section decompressed sizes, properly page-aligning memory regions during load.
  - Increased maximum image file size to 256 MiB and memory size to 512 MiB to accommodate commercial game executables.
  - Added path normalization (`Pfs0Archive::normalize_filename`) stripping leading slashes, `./`, `exefs/`, nulls, and whitespace during file extraction and lookup.
  - Expanded ExeFS candidate executable names (`rtld`, `nnrtld`, `main`, `program`, `application`, `sdk0`, etc.) and prevent silent suppression of NSO parsing errors during fallback archive scanning.
  - Updated `open_exefs()` so non-executable logo partitions (containing `NintendoLogo.png` / `StartupMovie.gif`) do not overwrite or mask primary ExeFS decryption errors.
  - Enhanced ExeFS error reporting to detail archive file inventory (filenames, sizes, magic headers) when no valid modules are detected.
- **Previous Release**: `v0.28.16-dev` (versionCode 56).
  - Resolved `invalid loader input: invalid PFS0/HFS0 magic` on Hierarchical SHA-256 PartitionFS (ExeFS) and CNMT sections.
  - Implemented `NcaFsHeader::data_region()` to parse data layer offsets from `HierarchicalSha256Data` (skipping layer 0 SHA-256 hash block tables) and IVFC levels.
  - Updated `open_exefs()` and `parse_cnmt()` to slice decrypted payloads by `data_region()` to locate the inner PFS0 archive.
  - Updated `decrypt_section()` candidate key testing to verify magic at `data_region.offset` using the corresponding offset CTR counter.
  - Added fallback in `ExeFsProgram::parse()` and `parse_cnmt()` to scan for `PFS0`/`HFS0` at 0x200-aligned sector boundaries in un-sliced payloads.
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

6. **Phase 1 (M5A) Delivered - NCA3 Decryption, RomFS & ExeFS Integration**:
   - Clean-room RomFS parser supporting hierarchical directories, file tables, and IVFC (Hierarchical Integrity) level extraction in `crates/loader/src/romfs.rs`.
   - Clean-room LZ4 block decompression in `crates/loader/src/lz4.rs` for retail compressed NSO modules.
   - NSO loader now handles compressed `.text`, `.rodata`, `.data` sections and verifies SHA-256 digests.
   - Multi-module ExeFS program loading and dynamic relocation support integrated with NPDM stack sizing in `Session::from_exefs`.
   - Full NCA3 container execution in `Session::from_nca`: header XTS decryption, integrity verification, ExeFS module loading, and RomFS mounting into `fsp-srv`.
   - CLI command `run-nca` in `crates/cli` for running decrypted/encrypted NCA game programs directly.

7. **Phase 2 (M5B) Delivered - AArch64 Floating Point & NEON Vector Engine**:
   - Extended `Cpu` with 32 128-bit vector registers `v: [u128; 32]`, `fpcr`, and `fpsr`.
   - Scalar FP arithmetic: `FADD`, `FSUB`, `FMUL`, `FDIV`, `FMAX`, `FMIN`, `FMAXNM`, `FMINNM`, `FNMUL`.
   - Scalar FP unary ops: `FMOV`, `FABS`, `FNEG`, `FSQRT`.
   - Fused multiply-add/sub: `FMADD`, `FMSUB`, `FNMADD`, `FNMSUB`.
   - Float comparison: `FCMP` (register and zero), updating architectural NZCV condition flags.
   - Conversions: `FCVT` (f32 <-> f64), integer conversions (`FCVTZS`, `FCVTZU`, `SCVTF`, `UCVTF`), and general/FP moves (`FMOV`).
   - SIMD & FP memory operations: `LDR` / `STR` for S (32-bit), D (64-bit), and Q (128-bit) registers with immediate scaling.
   - SIMD & FP pair operations: `LDP` / `STP` for S, D, and Q registers with pre/post-indexing.
   - NEON 128-bit vector bitwise logic: `AND`, `BIC`, `ORR`, `ORN`, and `EOR`.

8. **NSP Package Container & Decryption Engine**:
   - Clean-room NSP (Nintendo Submission Package / PFS0) parser in [`crates/loader/src/nsp.rs`](file:///root/projects/emulator/crates/loader/src/nsp.rs).
   - Ticket (`.tik`) parser in [`crates/loader/src/ticket.rs`](file:///root/projects/emulator/crates/loader/src/ticket.rs) supporting RSA-2048, RSA-4096, and ECDSA signature types.
   - Common title key unwrapping via AES-128-ECB decryption using `titlekek` matching master key revision; dynamic key registration into `KeyFile`.
   - CNMT (`PackagedContentMeta`) parser in [`crates/loader/src/cnmt.rs`](file:///root/projects/emulator/crates/loader/src/cnmt.rs) extracting Content IDs and mapping Program/Control/LegalInformation NCAs from `.cnmt.nca`.
   - Full package execution via `Session::from_nsp`: ticket import, CNMT program resolution, NCA3 header XTS decryption, ExeFS CTR decryption, RomFS mounting, and guest code execution.
   - CLI command `run-nsp` in `crates/cli` with `--keys` and `--titlekeys` support. Details: [`docs/nsp-parser.md`](docs/nsp-parser.md).

---

## Canonical Pokémon Legends: Z-A Execution Roadmap

The complete clean-room master plan for booting and running Pokémon Legends: Z-A on the Realme GT 7 Dream Edition (Arm Immortalis-G720) is documented in [`docs/pokemon-legends-za-plan.md`](file:///root/projects/emulator/docs/pokemon-legends-za-plan.md).

The implementation roadmap is structured into eight concrete milestones:
1. **Phase 1 (M5A) - NCA3 Decryption, Key Hierarchy & RomFS**: [DELIVERED]
2. **Phase 2 (M5B) - AArch64 Floating Point & NEON Vectors**: [DELIVERED]
3. **Phase 3 (M6A) - Horizon OS Multi-Core Kernel & Services**: 3-core guest scheduler simulation, user-space futex arbitration (`svcWaitProcessWideKeyAtomic`), full `fsp-srv`, `vi:m`/`nvnflinger` BufferQueue presentation, `hid` controller ring buffers, `audren:u` audio mixing, and `nvdrv` GPFIFO submission.
4. **Phase 4 (M6B) - Maxwell 3D GPFIFO Engine & State**: GPFIFO pushbuffer parsing (methods, gather buffers, call/return), full B197 3D state machine (16 VBs, 32 attrs, two-sided stencil, MRTs, independent blend), TIC/TSC samplers, and syncpoint semaphores.
5. **Phase 5 (M6C) - Maxwell SM53 Shader Translator & Compute**: Full SM53 opcodes, typed SSA IR, compute workgroup shared memory (`LDS`/`STS`), `BAR.SYNC`, perspective-correct fragment interpolation (`IPA`), and SPIR-V 1.3 emission.
6. **Phase 6 (M7A) - Host Vulkan Immortalis-G720 Renderer**: Compute-to-draw skinning barriers (preventing post-menu black screens), EDS capability-gated static pipeline fallback, GPU compute BCn decompression to ASTC/RGBA with retained GOB backing, and Reverse-Z depth clamping.
7. **Phase 7 (M7B) - Android ARM64 Native Code Execution (NCE)**: Host direct execution on Dimensity 9400e Cortex cores, 16 KiB-aligned virtual address space, and fast SVC signal interception (>60 MIPS/core).
8. **Phase 8 (M8) - Pokémon Legends: Z-A Acceptance Campaign**: Staged verification (Title Boot -> Menu -> Post-Menu Intro -> Lumiose City 3D Exploration -> Save/Load Durability at 30 FPS).

---

## Next Planned Steps (In Exact Priority Order)

1. **Horizon VI BufferQueue & Presentation Pipeline (Phase 3 / M6A)**:
   - Implement clean-room `nvnflinger` BufferQueue state machine (`Connect`, `RequestBuffer`, `QueueBuffer`, `DequeueBuffer`).
   - Connect buffer presentation to the Android Vulkan swapchain / `ANativeWindow`.

2. **NV Channel GPFIFO Submission Pipeline (Phase 4 / M6B)**:
   - Support `NVIOC_CHANNEL_SUBMIT_GPFIFO` command buffer submission.
   - Wire guest command stream decoding directly into the host Vulkan command processor.

3. **Multi-Thread Futex Synchronization (Phase 3 / M6A)**:
   - Implement monotonic ticket arbitration for `svcWaitProcessWideKeyAtomic` and `svcSignalProcessWideKey` preventing post-menu deadlock in Trinity Engine.
