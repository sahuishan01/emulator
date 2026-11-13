//! Clean-room parser for the decrypted 0x400-byte NCA3 header.
//!
//! Raw NCA files encrypt their header. `parse_decrypted` accepts an already
//! decrypted main header; `decrypt_nca3_header` handles only the encrypted
//! 0xC00-byte header area. Neither function authenticates signatures or hashes
//! nor decrypts section payloads.

use crate::LoadError;
use emulator_crypto::aes128::aes128_xts_decrypt;
use emulator_crypto::keys::KeyFile;

pub const NCA_HEADER_SIZE: usize = 0x400;
pub const NCA_ENCRYPTED_HEADER_SIZE: usize = 0xC00;
pub const NCA_MEDIA_UNIT_SIZE: u64 = 0x200;
pub const NCA_SECTION_COUNT: usize = 4;
pub const NCA_FS_HEADER_SIZE: usize = 0x200;
const NCA_MAGIC_OFFSET: usize = 0x200;
const SECTION_TABLE_OFFSET: usize = 0x240;
const SECTION_HASHES_OFFSET: usize = 0x280;

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

/// One active NCA section. Offsets are absolute byte ranges in the archive;
/// `end` is exclusive. `fs_header_hash` is the stored SHA-256 value and is not
/// verified until the section header bytes are available.
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
}

/// Decrypted main header and the four corresponding filesystem headers.
/// XTS provides confidentiality only; no signature or SHA-256 authentication
/// is verified by this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NcaDecryptedHeader {
    pub metadata: NcaHeader,
    pub fs_headers: [[u8; NCA_FS_HEADER_SIZE]; NCA_SECTION_COUNT],
}

/// Decrypt the NCA3 0xC00-byte header using six independent 0x200-byte XTS
/// data units. Nintendo encodes each sector index as a big-endian 128-bit
/// tweak. The required 32-byte `header_key` is looked up without exposing its
/// value in diagnostics.
pub fn decrypt_nca3_header(
    archive: &[u8],
    keys: &KeyFile,
) -> Result<NcaDecryptedHeader, LoadError> {
    if archive.len() < NCA_ENCRYPTED_HEADER_SIZE {
        return Err(LoadError::Invalid("NCA3 encrypted header is truncated"));
    }
    let key = keys.header_key().ok_or(LoadError::Invalid(
        "NCA3 header key is missing or malformed",
    ))?;
    let mut plaintext = [0; NCA_ENCRYPTED_HEADER_SIZE];
    plaintext.copy_from_slice(&archive[..NCA_ENCRYPTED_HEADER_SIZE]);
    for sector_index in 0..NCA_ENCRYPTED_HEADER_SIZE / NCA_MEDIA_UNIT_SIZE as usize {
        let start = sector_index * NCA_MEDIA_UNIT_SIZE as usize;
        let end = start + NCA_MEDIA_UNIT_SIZE as usize;
        let tweak = (sector_index as u128).to_be_bytes();
        aes128_xts_decrypt(key, tweak, &mut plaintext[start..end])
            .map_err(|_| LoadError::Invalid("NCA3 XTS sector has an invalid length"))?;
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
    /// Parse an NCA3 plaintext header, normally produced by a later decryption
    /// layer. `archive_size` must be the length of the original NCA byte stream.
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

        let distribution_type = NcaDistributionType::try_from(main[0x204])?;
        let content_type = NcaContentType::try_from(main[0x205])?;
        let key_generation_old = main[0x206];
        if key_generation_old > 2 {
            return Err(LoadError::Invalid("NCA3 legacy key generation is reserved"));
        }
        let key_area_encryption_key_index = main[0x207];
        if key_area_encryption_key_index > 2 {
            return Err(LoadError::Invalid("NCA3 key-area index is reserved"));
        }

        let content_size = u64::from_le_bytes(main[0x208..0x210].try_into().unwrap());
        if content_size < NCA_ENCRYPTED_HEADER_SIZE as u64
            || content_size % NCA_MEDIA_UNIT_SIZE != 0
            || content_size != archive_size
        {
            return Err(LoadError::Invalid(
                "NCA3 content size does not match the archive",
            ));
        }

        let program_id = u64::from_le_bytes(main[0x210..0x218].try_into().unwrap());
        let content_index = u32::from_le_bytes(main[0x218..0x21c].try_into().unwrap());
        let sdk_addon_version = u32::from_le_bytes(main[0x21c..0x220].try_into().unwrap());
        let key_generation = main[0x220];
        let signature_key_generation = main[0x221];
        if main[0x222..0x230].iter().any(|byte| *byte != 0) {
            return Err(LoadError::Invalid("NCA3 reserved header bytes are nonzero"));
        }

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
            let reserved = &main[entry_offset + 8..entry_offset + 0x10];
            if reserved.iter().any(|byte| *byte != 0) {
                return Err(LoadError::Invalid(
                    "NCA3 section entry reserved bytes are nonzero",
                ));
            }

            let hash_offset = SECTION_HASHES_OFFSET + index * 0x20;
            let hash = &main[hash_offset..hash_offset + 0x20];
            if start_units == 0 && end_units == 0 {
                if hash.iter().any(|byte| *byte != 0) {
                    return Err(LoadError::Invalid(
                        "NCA3 inactive section has a nonzero header hash",
                    ));
                }
                continue;
            }
            if start_units == 0 || end_units <= start_units {
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
            if start < NCA_ENCRYPTED_HEADER_SIZE as u64 || end > content_size {
                return Err(LoadError::Invalid(
                    "NCA3 section range exceeds content bounds",
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
        })
    }
}
