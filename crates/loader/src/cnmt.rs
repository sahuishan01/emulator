//! Clean-room CNMT (Content Meta / PackagedContentMeta) parser.
//!
//! Nintendo Switch applications package their content metadata in CNMT files inside
//! a Meta NCA (Section 0 PartitionFS). The CNMT describes all content records
//! (Program, Control, LegalInformation, etc.) including their Content IDs, types,
//! SHA-256 hashes, and sizes.

use crate::LoadError;

pub const CNMT_HEADER_SIZE: usize = 0x20;
pub const PACKAGED_CONTENT_INFO_SIZE: usize = 0x38;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ContentMetaType {
    SystemProgram = 0x80,
    SystemData = 0x81,
    SystemUpdate = 0x82,
    BootImagePackage = 0x83,
    BootImages = 0x84,
    Application = 0x1,
    Patch = 0x2,
    AddOnContent = 0x3,
    Delta = 0x4,
}

impl TryFrom<u8> for ContentMetaType {
    type Error = LoadError;

    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0x80 => Ok(Self::SystemProgram),
            0x81 => Ok(Self::SystemData),
            0x82 => Ok(Self::SystemUpdate),
            0x83 => Ok(Self::BootImagePackage),
            0x84 => Ok(Self::BootImages),
            0x01 => Ok(Self::Application),
            0x02 => Ok(Self::Patch),
            0x03 => Ok(Self::AddOnContent),
            0x04 => Ok(Self::Delta),
            _ => Err(LoadError::Invalid("unrecognized CNMT content meta type")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CnmtContentType {
    Meta = 0,
    Program = 1,
    Data = 2,
    Control = 3,
    HtmlDocument = 4,
    LegalInformation = 5,
    Delta = 6,
}

impl TryFrom<u8> for CnmtContentType {
    type Error = LoadError;

    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Self::Meta),
            1 => Ok(Self::Program),
            2 => Ok(Self::Data),
            3 => Ok(Self::Control),
            4 => Ok(Self::HtmlDocument),
            5 => Ok(Self::LegalInformation),
            6 => Ok(Self::Delta),
            _ => Err(LoadError::Invalid("unrecognized CNMT content type")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagedContentInfo {
    pub hash: [u8; 32],
    pub content_id: [u8; 16],
    pub size: u64,
    pub content_type: CnmtContentType,
    pub id_offset: u8,
}

impl PackagedContentInfo {
    /// Formatted 32-character lowercase hex content ID (matches `<id>.nca`).
    pub fn content_id_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(32);
        for byte in &self.content_id {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }

    /// Corresponding archive file name inside the NSP (e.g. `<content_id>.nca`).
    pub fn nca_filename(&self) -> String {
        format!("{}.nca", self.content_id_hex())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cnmt {
    pub title_id: u64,
    pub version: u32,
    pub meta_type: ContentMetaType,
    pub required_download_system_version: u32,
    pub contents: Vec<PackagedContentInfo>,
}

impl Cnmt {
    /// Parse a raw binary `.cnmt` file.
    pub fn parse(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() < CNMT_HEADER_SIZE {
            return Err(LoadError::Invalid("CNMT header is truncated"));
        }

        let title_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let meta_type = ContentMetaType::try_from(bytes[12])?;
        let ext_header_size = u16::from_le_bytes(bytes[14..16].try_into().unwrap()) as usize;
        let content_count = u16::from_le_bytes(bytes[16..18].try_into().unwrap()) as usize;
        let required_download_system_version =
            u32::from_le_bytes(bytes[24..28].try_into().unwrap());

        let records_start = CNMT_HEADER_SIZE
            .checked_add(ext_header_size)
            .ok_or(LoadError::LimitExceeded)?;
        let records_total_size = content_count
            .checked_mul(PACKAGED_CONTENT_INFO_SIZE)
            .ok_or(LoadError::LimitExceeded)?;
        let records_end = records_start
            .checked_add(records_total_size)
            .ok_or(LoadError::LimitExceeded)?;

        if bytes.len() < records_end {
            return Err(LoadError::Invalid("CNMT content records table is truncated"));
        }

        let mut contents = Vec::with_capacity(content_count);
        for i in 0..content_count {
            let offset = records_start + i * PACKAGED_CONTENT_INFO_SIZE;
            let record = &bytes[offset..offset + PACKAGED_CONTENT_INFO_SIZE];

            let mut hash = [0u8; 32];
            hash.copy_from_slice(&record[0..32]);

            let mut content_id = [0u8; 16];
            content_id.copy_from_slice(&record[32..48]);

            // Size is 6 bytes (little-endian 48-bit unsigned integer)
            let mut size_bytes = [0u8; 8];
            size_bytes[..6].copy_from_slice(&record[48..54]);
            let size = u64::from_le_bytes(size_bytes);

            let content_type = CnmtContentType::try_from(record[54])?;
            let id_offset = record[55];

            contents.push(PackagedContentInfo {
                hash,
                content_id,
                size,
                content_type,
                id_offset,
            });
        }

        Ok(Self {
            title_id,
            version,
            meta_type,
            required_download_system_version,
            contents,
        })
    }

    /// Locate the Program NCA content record.
    pub fn find_program_record(&self) -> Option<&PackagedContentInfo> {
        self.contents
            .iter()
            .find(|c| c.content_type == CnmtContentType::Program)
    }

    /// Locate the Control NCA content record.
    pub fn find_control_record(&self) -> Option<&PackagedContentInfo> {
        self.contents
            .iter()
            .find(|c| c.content_type == CnmtContentType::Control)
    }

    /// Locate the LegalInformation NCA content record.
    pub fn find_legal_info_record(&self) -> Option<&PackagedContentInfo> {
        self.contents
            .iter()
            .find(|c| c.content_type == CnmtContentType::LegalInformation)
    }
}
