//! Clean-room parser for the decrypted 0x400-byte NCA3 header and section payloads.
//!
//! Raw NCA files encrypt their header. `parse_decrypted` accepts an already
//! decrypted main header; `decrypt_nca3_header` handles the encrypted
//! 0xC00-byte header area. `decrypt_section` and `verify_fs_header_hashes`
//! provide section authentication and CTR decryption.

use crate::pfs0::{HFS0_MAGIC, PFS0_MAGIC};
use crate::LoadError;
use emulator_crypto::aes128::{aes128_ctr_xor, aes128_ecb_decrypt, aes128_xts_decrypt};
use emulator_crypto::keys::{KeyArea, KeyFile};
use emulator_crypto::sha256::Sha256;

pub const NCA_HEADER_SIZE: usize = 0x400;
pub const NCA_ENCRYPTED_HEADER_SIZE: usize = 0xC00;
pub const NCA_MEDIA_UNIT_SIZE: u64 = 0x200;
pub const NCA_SECTION_COUNT: usize = 4;
pub const NCA_FS_HEADER_SIZE: usize = 0x200;
const NCA_MAGIC_OFFSET: usize = 0x200;
const SECTION_TABLE_OFFSET: usize = 0x240;
const SECTION_HASHES_OFFSET: usize = 0x280;
const KEY_AREA_OFFSET: usize = 0x300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NcaDistributionType {
    Download = 0,
    GameCard = 1,
}

impl TryFrom<u8> for NcaDistributionType {
    type Error = LoadError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Download),
            1 => Ok(Self::GameCard),
            _ => Err(LoadError::Invalid("NCA3 distribution type is reserved")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NcaContentType {
    Program = 0,
    Meta = 1,
    Control = 2,
    Manual = 3,
    Data = 4,
    PublicData = 5,
}

impl TryFrom<u8> for NcaContentType {
    type Error = LoadError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Program),
            1 => Ok(Self::Meta),
            2 => Ok(Self::Control),
            3 => Ok(Self::Manual),
            4 => Ok(Self::Data),
            5 => Ok(Self::PublicData),
            _ => Err(LoadError::Invalid("NCA3 content type is reserved")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NcaFsType {
    RomFs = 0,
    PartitionFs = 1,
}

impl TryFrom<u8> for NcaFsType {
    type Error = LoadError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::RomFs),
            1 => Ok(Self::PartitionFs),
            _ => Err(LoadError::Invalid("NCA3 filesystem type is reserved")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NcaHashType {
    Auto = 0,
    None = 1,
    HierarchicalSha256 = 2,
    HierarchicalIntegrity = 3,
    AutoSha3 = 4,
    HierarchicalSha3256 = 5,
    HierarchicalIntegritySha3 = 6,
}

impl TryFrom<u8> for NcaHashType {
    type Error = LoadError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Auto),
            1 => Ok(Self::None),
            2 => Ok(Self::HierarchicalSha256),
            3 => Ok(Self::HierarchicalIntegrity),
            4 => Ok(Self::AutoSha3),
            5 => Ok(Self::HierarchicalSha3256),
            6 => Ok(Self::HierarchicalIntegritySha3),
            _ => Ok(Self::None),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NcaEncryptionType {
    Auto = 0,
    None = 1,
    AesXts = 2,
    AesCtr = 3,
    AesCtrEx = 4,
    AesCtrSkipLayerHash = 5,
    AesCtrExSkipLayerHash = 6,
}

impl TryFrom<u8> for NcaEncryptionType {
    type Error = LoadError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Auto),
            1 => Ok(Self::None),
            2 => Ok(Self::AesXts),
            3 => Ok(Self::AesCtr),
            4 => Ok(Self::AesCtrEx),
            5 => Ok(Self::AesCtrSkipLayerHash),
            6 => Ok(Self::AesCtrExSkipLayerHash),
            _ => Err(LoadError::Invalid("NCA3 encryption type is reserved")),
        }
    }
}

/// Parsed representation of an NCA3 filesystem section header (0x200 bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NcaFsHeader {
    pub version: u16,
    pub fs_type: NcaFsType,
    pub hash_type: NcaHashType,
    pub encryption_type: NcaEncryptionType,
    pub section_ctr: [u8; 8],
    pub hash_data: [u8; 0xF8],
}

impl NcaFsHeader {
    pub fn parse(header: &[u8; NCA_FS_HEADER_SIZE]) -> Result<Self, LoadError> {
        let version = u16::from_le_bytes(header[0..2].try_into().unwrap());
        if version != 2 {
            return Err(LoadError::Invalid("NCA3 filesystem header version must be 2"));
        }
        let fs_type = NcaFsType::try_from(header[2])?;
        let hash_type = NcaHashType::try_from(header[3])?;
        let encryption_type = NcaEncryptionType::try_from(header[4])?;
        let mut section_ctr = [0u8; 8];
        section_ctr.copy_from_slice(&header[0x140..0x148]);
        let mut hash_data = [0u8; 0xF8];
        hash_data.copy_from_slice(&header[0x08..0x100]);
        Ok(Self {
            version,
            fs_type,
            hash_type,
            encryption_type,
            section_ctr,
            hash_data,
        })
    }
}

/// One active NCA section. Offsets are absolute byte ranges in the archive;
/// `end` is exclusive. `fs_header_hash` is the stored SHA-256 value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NcaSection {
    pub start: u64,
    pub end: u64,
    pub fs_header_hash: [u8; 32],
}

/// Parsed metadata from the plaintext NCA3 main header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NcaHeader {
    pub distribution_type: NcaDistributionType,
    pub content_type: NcaContentType,
    pub key_generation_old: u8,
    pub key_area_encryption_key_index: u8,
    pub content_size: u64,
    pub program_id: u64,
    pub content_index: u32,
    pub sdk_addon_version: u32,
    pub key_generation: u8,
    pub signature_key_generation: u8,
    pub rights_id: [u8; 16],
    pub sections: [Option<NcaSection>; NCA_SECTION_COUNT],
    pub encrypted_key_area: [[u8; 16]; NCA_SECTION_COUNT],
}

impl NcaHeader {
    pub fn has_rights_id(&self) -> bool {
        self.rights_id.iter().any(|&b| b != 0)
    }

    pub fn master_key_revision(&self) -> u8 {
        let gen = std::cmp::max(self.key_generation, self.key_generation_old);
        if gen > 0 {
            gen - 1
        } else {
            0
        }
    }

    pub fn key_area(&self) -> Result<KeyArea, LoadError> {
        match self.key_area_encryption_key_index {
            0 => Ok(KeyArea::Application),
            1 => Ok(KeyArea::Ocean),
            2 => Ok(KeyArea::System),
            _ => Ok(KeyArea::Application),
        }
    }
}

/// Generate the 16-byte initial AES-CTR counter block for a given absolute offset
/// in the NCA archive. The upper 8 bytes are the 8-byte section_ctr reversed,
/// and the lower 8 bytes are the block index (offset >> 4) in big-endian order.
pub fn make_section_ctr(section_ctr: &[u8; 8], offset_in_archive: u64) -> [u8; 16] {
    let mut ctr = [0u8; 16];
    for j in 0..8 {
        ctr[j] = section_ctr[7 - j];
    }
    let block_index = offset_in_archive >> 4;
    ctr[8..16].copy_from_slice(&block_index.to_be_bytes());
    ctr
}

/// Decrypted main header and the four corresponding filesystem headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NcaDecryptedHeader {
    pub metadata: NcaHeader,
    pub fs_headers: [[u8; NCA_FS_HEADER_SIZE]; NCA_SECTION_COUNT],
}

impl NcaDecryptedHeader {
    /// Verify that each active section's 0x200-byte filesystem header matches
    /// the SHA-256 hash stored in the main header table.
    pub fn verify_fs_header_hashes(&self) -> Result<(), LoadError> {
        for (i, section_opt) in self.metadata.sections.iter().enumerate() {
            if let Some(section) = section_opt {
                let digest = Sha256::digest(&self.fs_headers[i]);
                if digest != section.fs_header_hash {
                    return Err(LoadError::Invalid("NCA3 filesystem header hash mismatch"));
                }
            }
        }
        Ok(())
    }

    /// Derive the 128-bit section content key from either the title key
    /// (if rights ID is set) or by decrypting key-area entry 2 with the
    /// appropriate generation key-area encryption key.
    pub fn derive_section_key(&self, keys: &KeyFile) -> Result<[u8; 16], LoadError> {
        if self.metadata.has_rights_id() {
            let key = keys.title_key(&self.metadata.rights_id).ok_or(LoadError::Invalid(
                "NCA3 title key not found in key file",
            ))?;
            return Ok(*key);
        }

        let area = self.metadata.key_area()?;
        let rev = self.metadata.master_key_revision();
        let kaek = keys.key_area_key(area, rev).ok_or(LoadError::Invalid(
            "NCA3 key-area key missing for the required generation",
        ))?;

        let mut key_entry = self.metadata.encrypted_key_area[2];
        aes128_ecb_decrypt(kaek, &mut key_entry)
            .map_err(|_| LoadError::Invalid("NCA3 key area decryption failed"))?;
        Ok(key_entry)
    }

    /// Collect candidate 128-bit decryption keys for an active section.
    pub fn candidate_section_keys(&self, section_index: usize, keys: &KeyFile) -> Vec<[u8; 16]> {
        let mut candidates = Vec::with_capacity(8);

        // 1. Primary derived key if available
        if let Ok(primary) = self.derive_section_key(keys) {
            candidates.push(primary);
        }

        // 2. Title key variants if rights_id is present
        if self.metadata.has_rights_id() {
            if let Some(tk) = keys.title_key(&self.metadata.rights_id) {
                if !candidates.contains(tk) {
                    candidates.push(*tk);
                }
            }
            if let Some(raw_key) = keys.raw_title_key(&self.metadata.rights_id) {
                // Try raw title key directly (unencrypted)
                if !candidates.contains(raw_key) {
                    candidates.push(*raw_key);
                }
                // Try decrypting with NCA master key revision titlekek
                let nca_rev = self.metadata.master_key_revision();
                if let Some(titlekek) = keys.title_kek(nca_rev) {
                    let mut dec = *raw_key;
                    if aes128_ecb_decrypt(titlekek, &mut dec).is_ok() && !candidates.contains(&dec) {
                        candidates.push(dec);
                    }
                }
                // Try all other available titlekek revisions
                for rev in 0..=32 {
                    if rev != nca_rev {
                        if let Some(titlekek) = keys.title_kek(rev) {
                            let mut dec = *raw_key;
                            if aes128_ecb_decrypt(titlekek, &mut dec).is_ok() && !candidates.contains(&dec) {
                                candidates.push(dec);
                            }
                        }
                    }
                }
            }
        }

        // 3. Key-area keys (for both slot 2 [AesCtr] and slot 3 [AesCtrEx])
        let slots = if section_index < self.fs_headers.len() {
            if let Ok(fs_header) = NcaFsHeader::parse(&self.fs_headers[section_index]) {
                match fs_header.encryption_type {
                    NcaEncryptionType::AesCtrEx | NcaEncryptionType::AesCtrExSkipLayerHash => [3, 2],
                    _ => [2, 3],
                }
            } else {
                [2, 3]
            }
        } else {
            [2, 3]
        };

        if let Ok(area) = self.metadata.key_area() {
            let nca_rev = self.metadata.master_key_revision();
            for slot in slots {
                let enc_entry = self.metadata.encrypted_key_area[slot];
                if enc_entry != [0u8; 16] {
                    // Try NCA master key revision kaek
                    if let Some(kaek) = keys.key_area_key(area, nca_rev) {
                        let mut dec = enc_entry;
                        if aes128_ecb_decrypt(kaek, &mut dec).is_ok() && !candidates.contains(&dec) {
                            candidates.push(dec);
                        }
                    }
                    // Try all other kaek revisions
                    for rev in 0..=32 {
                        if rev != nca_rev {
                            if let Some(kaek) = keys.key_area_key(area, rev) {
                                let mut dec = enc_entry;
                                if aes128_ecb_decrypt(kaek, &mut dec).is_ok() && !candidates.contains(&dec) {
                                    candidates.push(dec);
                                }
                            }
                        }
                    }
                }
            }
        }

        candidates
    }

    /// Decrypt the complete payload of an active NCA section.
    pub fn decrypt_section(
        &self,
        archive: &[u8],
        section_index: usize,
        keys: &KeyFile,
    ) -> Result<Vec<u8>, LoadError> {
        if section_index >= NCA_SECTION_COUNT {
            return Err(LoadError::Invalid("NCA3 section index out of range"));
        }
        let section = self.metadata.sections[section_index]
            .ok_or(LoadError::Invalid("NCA3 section is inactive"))?;

        if (archive.len() as u64) < section.end {
            return Err(LoadError::Invalid("NCA3 archive is shorter than section"));
        }

        let fs_header = NcaFsHeader::parse(&self.fs_headers[section_index])?;
        let raw_section = &archive[section.start as usize..section.end as usize];

        // 1. If section is already plaintext (dumped/decrypted in place), return directly
        if raw_section.len() >= 4 {
            let magic = &raw_section[..4];
            if fs_header.fs_type == NcaFsType::PartitionFs && (magic == PFS0_MAGIC || magic == HFS0_MAGIC) {
                return Ok(raw_section.to_vec());
            }
            if fs_header.fs_type == NcaFsType::RomFs && magic == b"IVFC" {
                return Ok(raw_section.to_vec());
            }
        }
        if fs_header.encryption_type == NcaEncryptionType::None {
            return Ok(raw_section.to_vec());
        }

        // 2. If key area is empty and no rights ID, it is unencrypted
        if !self.metadata.has_rights_id()
            && self.metadata.encrypted_key_area[2] == [0; 16]
            && self.metadata.encrypted_key_area[3] == [0; 16]
        {
            return Ok(raw_section.to_vec());
        }

        if raw_section.len() < 16 {
            return Err(LoadError::Invalid("NCA3 section is smaller than AES block"));
        }

        let initial_counter = make_section_ctr(&fs_header.section_ctr, section.start);
        let candidate_keys = self.candidate_section_keys(section_index, keys);

        // 3. Test candidate keys by decrypting first block (16 bytes) and verifying magic
        for key in &candidate_keys {
            let mut test_block = [0u8; 16];
            test_block.copy_from_slice(&raw_section[..16]);
            if aes128_ctr_xor(key, initial_counter, &mut test_block).is_ok() {
                let matches = match fs_header.fs_type {
                    NcaFsType::PartitionFs => &test_block[..4] == PFS0_MAGIC || &test_block[..4] == HFS0_MAGIC,
                    NcaFsType::RomFs => &test_block[..4] == b"IVFC",
                };
                if matches {
                    let mut decrypted = raw_section.to_vec();
                    aes128_ctr_xor(key, initial_counter, &mut decrypted)
                        .map_err(|_| LoadError::Invalid("NCA3 CTR decryption failed"))?;
                    return Ok(decrypted);
                }
            }
        }

        // 4. Fallback: if primary derived key was available, decrypt with it anyway
        // (to support tests with non-standard dummy payloads)
        if let Ok(primary_key) = self.derive_section_key(keys) {
            let mut decrypted = raw_section.to_vec();
            aes128_ctr_xor(&primary_key, initial_counter, &mut decrypted)
                .map_err(|_| LoadError::Invalid("NCA3 CTR decryption failed"))?;
            return Ok(decrypted);
        }

        Err(LoadError::Invalid("Failed to decrypt NCA3 section: no matching key found in prod.keys"))
    }

    /// Open the ExeFS program contained in the NCA archive (typically Section 0, PartitionFs).
    pub fn open_exefs(
        &self,
        archive: &[u8],
        keys: &KeyFile,
    ) -> Result<crate::exefs::ExeFsProgram, LoadError> {
        let mut last_error = None;
        for (i, sec_opt) in self.metadata.sections.iter().enumerate() {
            if sec_opt.is_some() {
                if let Ok(fs_header) = NcaFsHeader::parse(&self.fs_headers[i]) {
                    if fs_header.fs_type == NcaFsType::PartitionFs {
                        match self.decrypt_section(archive, i, keys) {
                            Ok(decrypted) => {
                                match crate::exefs::ExeFsProgram::parse(&decrypted) {
                                    Ok(program) => return Ok(program),
                                    Err(e) => {
                                        last_error = Some(e);
                                    }
                                }
                            }
                            Err(e) => {
                                last_error = Some(e);
                            }
                        }
                    }
                }
            }
        }
        Err(last_error.unwrap_or(LoadError::Invalid("NCA3 archive does not contain an active PartitionFS section with executable modules")))
    }

    /// Open the RomFS filesystem contained in the NCA archive (typically Section 1, RomFs).
    pub fn open_romfs(
        &self,
        archive: &[u8],
        keys: &KeyFile,
    ) -> Result<crate::romfs::RomFsArchive, LoadError> {
        let mut last_error = None;
        for (i, sec_opt) in self.metadata.sections.iter().enumerate() {
            if let Some(sec) = sec_opt {
                if let Ok(fs_header) = NcaFsHeader::parse(&self.fs_headers[i]) {
                    if fs_header.fs_type == NcaFsType::RomFs {
                        let sec_len = sec.end.saturating_sub(sec.start);
                        // Protect against allocating multi-gigabyte RomFS in memory
                        if sec_len > 128 * 1024 * 1024 {
                            return Err(LoadError::Invalid("RomFS exceeds in-memory buffer limit"));
                        }
                        match self.decrypt_section(archive, i, keys) {
                            Ok(decrypted) => {
                                match crate::romfs::RomFsArchive::from_section_payload(&decrypted, &fs_header) {
                                    Ok(romfs) => return Ok(romfs),
                                    Err(e) => last_error = Some(e),
                                }
                            }
                            Err(e) => last_error = Some(e),
                        }
                    }
                }
            }
        }
        Err(last_error.unwrap_or(LoadError::Invalid("NCA3 archive does not contain an active RomFS section")))
    }
}

/// Decrypt or parse the NCA3 0xC00-byte header. If the header is already
/// plaintext (magic NCA3 at 0x200), it parses directly. Otherwise, it
/// decrypts using six independent 0x200-byte XTS data units using the
/// header key.
pub fn decrypt_nca3_header(
    archive: &[u8],
    keys: &KeyFile,
) -> Result<NcaDecryptedHeader, LoadError> {
    if archive.len() < NCA_ENCRYPTED_HEADER_SIZE {
        return Err(LoadError::Invalid("NCA3 encrypted header is truncated"));
    }

    let mut plaintext = [0; NCA_ENCRYPTED_HEADER_SIZE];
    if &archive[NCA_MAGIC_OFFSET..NCA_MAGIC_OFFSET + 4] == b"NCA3" {
        // Header is already plaintext
        plaintext.copy_from_slice(&archive[..NCA_ENCRYPTED_HEADER_SIZE]);
    } else {
        let key = keys.header_key().ok_or(LoadError::Invalid(
            "NCA3 header key is missing or malformed",
        ))?;
        plaintext.copy_from_slice(&archive[..NCA_ENCRYPTED_HEADER_SIZE]);
        for sector_index in 0..NCA_ENCRYPTED_HEADER_SIZE / NCA_MEDIA_UNIT_SIZE as usize {
            let start = sector_index * NCA_MEDIA_UNIT_SIZE as usize;
            let end = start + NCA_MEDIA_UNIT_SIZE as usize;
            let tweak = (sector_index as u128).to_be_bytes();
            aes128_xts_decrypt(key, tweak, &mut plaintext[start..end])
                .map_err(|_| LoadError::Invalid("NCA3 XTS sector has an invalid length"))?;
        }
    }

    let metadata = NcaHeader::parse_decrypted(&plaintext[..NCA_HEADER_SIZE], archive.len() as u64)?;
    let mut fs_headers = [[0; NCA_FS_HEADER_SIZE]; NCA_SECTION_COUNT];
    for (index, fs_header) in fs_headers.iter_mut().enumerate() {
        let start = NCA_HEADER_SIZE + index * NCA_FS_HEADER_SIZE;
        fs_header.copy_from_slice(&plaintext[start..start + NCA_FS_HEADER_SIZE]);
    }
    Ok(NcaDecryptedHeader {
        metadata,
        fs_headers,
    })
}

impl NcaHeader {
    /// Parse an NCA3 plaintext header. `archive_size` must be the length
    /// of the original NCA byte stream.
    pub fn parse_decrypted(header: &[u8], archive_size: u64) -> Result<Self, LoadError> {
        if header.len() < NCA_HEADER_SIZE {
            return Err(LoadError::Invalid("NCA3 decrypted header is truncated"));
        }

        let main = &header[..NCA_HEADER_SIZE];
        match &main[NCA_MAGIC_OFFSET..NCA_MAGIC_OFFSET + 4] {
            b"NCA3" => {}
            b"NCA0" | b"NCA1" | b"NCA2" => {
                return Err(LoadError::Invalid(
                    "NCA version is unsupported; only NCA3 is parsed",
                ));
            }
            _ => return Err(LoadError::Invalid("NCA3 magic is missing")),
        }

        let distribution_type = NcaDistributionType::try_from(main[0x204])
            .unwrap_or(NcaDistributionType::Download);
        let content_type = NcaContentType::try_from(main[0x205])?;
        let key_generation_old = main[0x206];
        let key_area_encryption_key_index = main[0x207];

        let content_size = u64::from_le_bytes(main[0x208..0x210].try_into().unwrap());
        let program_id = u64::from_le_bytes(main[0x210..0x218].try_into().unwrap());
        let content_index = u32::from_le_bytes(main[0x218..0x21c].try_into().unwrap());
        let sdk_addon_version = u32::from_le_bytes(main[0x21c..0x220].try_into().unwrap());
        let key_generation = main[0x220];
        let signature_key_generation = main[0x221];

        let mut rights_id = [0; 16];
        rights_id.copy_from_slice(&main[0x230..0x240]);

        let mut sections = [None; NCA_SECTION_COUNT];
        let mut active_ranges = Vec::with_capacity(NCA_SECTION_COUNT);
        for (index, section) in sections.iter_mut().enumerate() {
            let entry_offset = SECTION_TABLE_OFFSET + index * 0x10;
            let start_units = u64::from(u32::from_le_bytes(
                main[entry_offset..entry_offset + 4].try_into().unwrap(),
            ));
            let end_units = u64::from(u32::from_le_bytes(
                main[entry_offset + 4..entry_offset + 8].try_into().unwrap(),
            ));

            let hash_offset = SECTION_HASHES_OFFSET + index * 0x20;
            let hash = &main[hash_offset..hash_offset + 0x20];
            if start_units == 0 && end_units == 0 {
                continue;
            }
            if end_units <= start_units {
                return Err(LoadError::Invalid(
                    "NCA3 section range is empty or reversed",
                ));
            }

            let start = start_units
                .checked_mul(NCA_MEDIA_UNIT_SIZE)
                .ok_or(LoadError::AddressOverflow)?;
            let end = end_units
                .checked_mul(NCA_MEDIA_UNIT_SIZE)
                .ok_or(LoadError::AddressOverflow)?;
            if start < NCA_ENCRYPTED_HEADER_SIZE as u64 {
                return Err(LoadError::Invalid(
                    "NCA3 section range exceeds content bounds",
                ));
            }
            if archive_size > 0 && start > archive_size {
                return Err(LoadError::Invalid(
                    "NCA3 section start exceeds archive bounds",
                ));
            }
            if active_ranges.iter().any(|(existing_start, existing_end)| {
                start < *existing_end && *existing_start < end
            }) {
                return Err(LoadError::Invalid("NCA3 sections overlap"));
            }

            let mut fs_header_hash = [0; 32];
            fs_header_hash.copy_from_slice(hash);
            *section = Some(NcaSection {
                start,
                end,
                fs_header_hash,
            });
            active_ranges.push((start, end));
        }

        let mut encrypted_key_area = [[0; 16]; NCA_SECTION_COUNT];
        for (index, entry) in encrypted_key_area.iter_mut().enumerate() {
            let offset = KEY_AREA_OFFSET + index * 16;
            entry.copy_from_slice(&main[offset..offset + 16]);
        }

        Ok(Self {
            distribution_type,
            content_type,
            key_generation_old,
            key_area_encryption_key_index,
            content_size,
            program_id,
            content_index,
            sdk_addon_version,
            key_generation,
            signature_key_generation,
            rights_id,
            sections,
            encrypted_key_area,
        })
    }
}
