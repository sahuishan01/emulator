# NCA3 header and section parser

The loader parses the **NCA3 main header**, decrypts encrypted headers via AES-128-XTS,
verifies section filesystem header SHA-256 hashes, and decrypts section payloads
using AES-128-CTR with key derivation from the key area or external title keys.

## Header Architecture

1. **Main Header (0x400 bytes)**:
   - Validates magic (`NCA3`), distribution type, content type, key-generation metadata,
     program ID, content index, and SDK add-on version.
   - Parses four section entries with exclusive media-unit byte ranges.
   - Enforces archive size alignment, bounds, and pairwise section overlap checks.
   - Reads the encrypted key area (`0x300..0x340`).

2. **Filesystem Section Headers (`0x400..0xC00`)**:
   - Each section has a dedicated 0x200-byte filesystem header (`NcaFsHeader`).
   - Parsed fields: `version`, `fs_type` (RomFs / PartitionFs), `hash_type`, `encryption_type` (None, Xts, Ctr, Bktr),
     and `section_ctr` (8 bytes at offset `0x140..0x148`).
   - Verified against the stored SHA-256 hashes in the main header via `verify_fs_header_hashes()`.

3. **Section CTR Cryptography**:
   - The 128-bit AES-CTR counter block is constructed via `make_section_ctr`:
     - Upper 8 bytes: the 8-byte `section_ctr` reversed (`section_ctr[7 - j]`).
     - Lower 8 bytes: the block index `(offset_in_archive >> 4)` in big-endian format.
   - Section payload decryption via `decrypt_section()` decrypts CTR-encrypted sections
     using the derived content key.

4. **Key Derivation Hierarchy**:
   - If `rights_id` is set (non-zero), look up `title_key(&rights_id)` from `KeyFile`.
   - If `rights_id` is zero:
     - Determine key area (`Application`, `Ocean`, or `System`) from `key_area_encryption_key_index`.
     - Determine master key revision: `max(key_generation, key_generation_old) - 1` (or 0).
     - Decrypt key-area slot 2 with `key_area_key` using AES-128-ECB.

All implementation tests use synthetic containers and vectors only.
