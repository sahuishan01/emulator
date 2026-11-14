//! Original PFS0 (Partition Filesystem / ExeFS / HFS0) parser.
//!
//! Nintendo Switch executables and packages (ExeFS, RomFS, NSP/XCI partitions)
//! use the PFS0 (or HFS0) format, consisting of a header, an array of file entries,
//! a string table containing UTF-8 filenames, and consecutive file data payloads.

use crate::LoadError;

pub const PFS0_MAGIC: &[u8; 4] = b"PFS0";
pub const HFS0_MAGIC: &[u8; 4] = b"HFS0";
pub const PFS0_HEADER_SIZE: usize = 0x10;
pub const PFS0_FILE_ENTRY_SIZE: usize = 0x18;
pub const MAX_PFS0_FILES: u32 = 1024;
pub const MAX_STRING_TABLE_SIZE: u32 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pfs0FileEntry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct Pfs0Archive<'a> {
    data: &'a [u8],
    files: Vec<Pfs0FileEntry>,
}

impl<'a> Pfs0Archive<'a> {
    /// Parses a PFS0 / HFS0 image without copying file payloads.
    pub fn parse(data: &'a [u8]) -> Result<Self, LoadError> {
        if data.len() < PFS0_HEADER_SIZE {
            return Err(LoadError::Invalid("PFS0 archive smaller than header"));
        }
        let magic = &data[0..4];
        if magic != PFS0_MAGIC && magic != HFS0_MAGIC {
            return Err(LoadError::Invalid("invalid PFS0/HFS0 magic"));
        }

        let num_files = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let string_table_size = u32::from_le_bytes(data[8..12].try_into().unwrap());
        // Bytes 12..16 are reserved/zero

        if num_files > MAX_PFS0_FILES {
            return Err(LoadError::LimitExceeded);
        }
        if string_table_size > MAX_STRING_TABLE_SIZE {
            return Err(LoadError::LimitExceeded);
        }

        let file_entries_size = (num_files as usize)
            .checked_mul(PFS0_FILE_ENTRY_SIZE)
            .ok_or(LoadError::LimitExceeded)?;
        let string_table_start = PFS0_HEADER_SIZE
            .checked_add(file_entries_size)
            .ok_or(LoadError::LimitExceeded)?;
        let string_table_end = string_table_start
            .checked_add(string_table_size as usize)
            .ok_or(LoadError::LimitExceeded)?;

        if string_table_end > data.len() {
            return Err(LoadError::Invalid("PFS0 header extends past archive EOF"));
        }

        let string_table = &data[string_table_start..string_table_end];
        let data_header_total = string_table_end as u64;

        let mut files = Vec::with_capacity(num_files as usize);

        for i in 0..num_files as usize {
            let entry_offset = PFS0_HEADER_SIZE + i * PFS0_FILE_ENTRY_SIZE;
            let file_data_offset = u64::from_le_bytes(data[entry_offset..entry_offset + 8].try_into().unwrap());
            let file_data_size = u64::from_le_bytes(data[entry_offset + 8..entry_offset + 16].try_into().unwrap());
            let name_offset = u32::from_le_bytes(data[entry_offset + 16..entry_offset + 20].try_into().unwrap()) as usize;
            // Bytes 20..24 are reserved/zero

            if name_offset >= string_table.len() {
                return Err(LoadError::Invalid("PFS0 filename offset out of string table bounds"));
            }

            // Find null terminator
            let null_pos = string_table[name_offset..]
                .iter()
                .position(|&b| b == 0)
                .ok_or(LoadError::Invalid("PFS0 filename missing null terminator"))?;
            let name_bytes = &string_table[name_offset..name_offset + null_pos];
            let raw_name = std::str::from_utf8(name_bytes)
                .map_err(|_| LoadError::Invalid("PFS0 filename is not valid UTF-8"))?;
            let name = raw_name.trim_matches('\0').trim().to_string();

            // Total file offset in archive
            let absolute_offset = data_header_total
                .checked_add(file_data_offset)
                .ok_or(LoadError::AddressOverflow)?;
            let absolute_end = absolute_offset
                .checked_add(file_data_size)
                .ok_or(LoadError::AddressOverflow)?;

            if absolute_end > data.len() as u64 {
                return Err(LoadError::Invalid("PFS0 file entry payload extends beyond archive EOF"));
            }

            files.push(Pfs0FileEntry {
                name,
                offset: absolute_offset,
                size: file_data_size,
            });
        }

        Ok(Self { data, files })
    }

    pub fn files(&self) -> &[Pfs0FileEntry] {
        &self.files
    }

    /// Normalizes a partition entry filename by stripping leading directory paths,
    /// slashes, whitespace, and null terminators.
    pub fn normalize_filename(name: &str) -> String {
        let mut s = name.trim();
        if let Some(pos) = s.find('\0') {
            s = &s[..pos];
        }
        let mut s = s.trim();
        if let Some(rest) = s.strip_prefix("./") {
            s = rest;
        }
        if let Some(rest) = s.strip_prefix('/') {
            s = rest;
        }
        if let Some(rest) = s.strip_prefix('\\') {
            s = rest;
        }
        if s.to_ascii_lowercase().starts_with("exefs/") {
            s = &s[6..];
        } else if s.to_ascii_lowercase().starts_with("exefs\\") {
            s = &s[6..];
        }
        s.to_string()
    }

    pub fn get_file(&self, name: &str) -> Option<&'a [u8]> {
        let clean_name = Self::normalize_filename(name);
        for entry in &self.files {
            let entry_clean = Self::normalize_filename(&entry.name);
            if entry.name == name
                || entry.name.eq_ignore_ascii_case(name)
                || entry_clean == clean_name
                || entry_clean.eq_ignore_ascii_case(&clean_name)
                || entry_clean
                    .strip_suffix(".nso")
                    .unwrap_or(&entry_clean)
                    .eq_ignore_ascii_case(
                        clean_name.strip_suffix(".nso").unwrap_or(&clean_name),
                    )
            {
                let start = entry.offset as usize;
                let end = start + (entry.size as usize);
                if end <= self.data.len() {
                    return Some(&self.data[start..end]);
                }
            }
        }
        None
    }
}
