//! Clean-room RomFS (Read-Only Filesystem) and IVFC parser.
//!
//! Nintendo Switch RomFS provides hierarchical read-only directory and file storage
//! for game assets. In NCA3 archives, RomFS sections typically use Hierarchical Integrity (IVFC).

use crate::nca::{NcaFsHeader, NcaHashType};
use crate::LoadError;
use std::collections::HashMap;

pub const ROMFS_HEADER_SIZE: usize = 0x50;
pub const IVFC_MAGIC: &[u8; 4] = b"IVFC";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomFsFileEntry {
    pub name: String,
    pub parent_offset: u32,
    pub sibling_offset: u32,
    pub data_offset: u64,
    pub data_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomFsDirEntry {
    pub name: String,
    pub parent_offset: u32,
    pub sibling_offset: u32,
    pub child_dir_offset: u32,
    pub child_file_offset: u32,
}

/// In-memory representation of an unpacked or mapped RomFS archive.
#[derive(Clone, Debug, Default)]
pub struct RomFsArchive {
    /// Maps normalized absolute path (e.g. "romfs:/data/model.bin" or "data/model.bin") to file offset and size
    files: HashMap<String, (u64, u64)>,
    /// Raw payload backing
    data_bytes: Vec<u8>,
    file_data_offset: u64,
}

impl RomFsArchive {
    /// Parses a RomFS filesystem directly from raw filesystem bytes.
    pub fn parse(image: &[u8]) -> Result<Self, LoadError> {
        if image.len() < ROMFS_HEADER_SIZE {
            return Err(LoadError::Invalid("RomFS image smaller than header"));
        }

        let header_size = u64::from_le_bytes(image[0x00..0x08].try_into().unwrap());
        if header_size != ROMFS_HEADER_SIZE as u64 {
            return Err(LoadError::Invalid("RomFS header size must be 0x50"));
        }

        let dir_hash_offset = u64::from_le_bytes(image[0x08..0x10].try_into().unwrap()) as usize;
        let dir_hash_size = u64::from_le_bytes(image[0x10..0x18].try_into().unwrap()) as usize;
        let dir_table_offset = u64::from_le_bytes(image[0x18..0x20].try_into().unwrap()) as usize;
        let dir_table_size = u64::from_le_bytes(image[0x20..0x28].try_into().unwrap()) as usize;
        let file_hash_offset = u64::from_le_bytes(image[0x28..0x30].try_into().unwrap()) as usize;
        let file_hash_size = u64::from_le_bytes(image[0x30..0x38].try_into().unwrap()) as usize;
        let file_table_offset = u64::from_le_bytes(image[0x38..0x40].try_into().unwrap()) as usize;
        let file_table_size = u64::from_le_bytes(image[0x40..0x48].try_into().unwrap()) as usize;
        let file_data_offset = u64::from_le_bytes(image[0x48..0x50].try_into().unwrap());

        let total_meta = file_data_offset as usize;
        if image.len() < total_meta {
            return Err(LoadError::Invalid("RomFS metadata exceeds image bounds"));
        }

        // Validate table bounds
        if dir_hash_offset + dir_hash_size > total_meta
            || dir_table_offset + dir_table_size > total_meta
            || file_hash_offset + file_hash_size > total_meta
            || file_table_offset + file_table_size > total_meta
        {
            return Err(LoadError::Invalid("RomFS table offsets exceed metadata bounds"));
        }

        let dir_table = &image[dir_table_offset..dir_table_offset + dir_table_size];
        let file_table = &image[file_table_offset..file_table_offset + file_table_size];

        // Parse directories and build path map
        let mut dir_paths: HashMap<u32, String> = HashMap::new();
        dir_paths.insert(0, String::new()); // root directory

        let mut offset = 0usize;
        while offset < dir_table.len() {
            if offset + 0x18 > dir_table.len() {
                break;
            }
            let parent_offset = u32::from_le_bytes(dir_table[offset..offset + 4].try_into().unwrap());
            let _sibling_offset = u32::from_le_bytes(dir_table[offset + 4..offset + 8].try_into().unwrap());
            let _child_dir_offset = u32::from_le_bytes(dir_table[offset + 8..offset + 12].try_into().unwrap());
            let _child_file_offset = u32::from_le_bytes(dir_table[offset + 12..offset + 16].try_into().unwrap());
            let _next_hash = u32::from_le_bytes(dir_table[offset + 16..offset + 20].try_into().unwrap());
            let name_length = u32::from_le_bytes(dir_table[offset + 20..offset + 24].try_into().unwrap()) as usize;

            let name_end = offset + 0x18 + name_length;
            if name_end > dir_table.len() {
                return Err(LoadError::Invalid("RomFS directory name exceeds table"));
            }
            let name = std::str::from_utf8(&dir_table[offset + 0x18..name_end])
                .unwrap_or("")
                .to_string();

            if offset != 0 {
                let parent_path = dir_paths.get(&parent_offset).cloned().unwrap_or_default();
                let full_dir = if parent_path.is_empty() {
                    name
                } else {
                    format!("{parent_path}/{name}")
                };
                dir_paths.insert(offset as u32, full_dir);
            }

            // Entries are padded to 4-byte alignment
            let padded_len = (name_length + 3) & !3;
            offset += 0x18 + padded_len;
        }

        // Parse files
        let mut files = HashMap::new();
        let mut file_offset = 0usize;
        while file_offset < file_table.len() {
            if file_offset + 0x20 > file_table.len() {
                break;
            }
            let parent_offset = u32::from_le_bytes(file_table[file_offset..file_offset + 4].try_into().unwrap());
            let _sibling_offset = u32::from_le_bytes(file_table[file_offset + 4..file_offset + 8].try_into().unwrap());
            let data_offset = u64::from_le_bytes(file_table[file_offset + 8..file_offset + 16].try_into().unwrap());
            let data_size = u64::from_le_bytes(file_table[file_offset + 16..file_offset + 24].try_into().unwrap());
            let _next_hash = u32::from_le_bytes(file_table[file_offset + 24..file_offset + 28].try_into().unwrap());
            let name_length = u32::from_le_bytes(file_table[file_offset + 28..file_offset + 32].try_into().unwrap()) as usize;

            let name_end = file_offset + 0x20 + name_length;
            if name_end > file_table.len() {
                return Err(LoadError::Invalid("RomFS file name exceeds table"));
            }
            let name = std::str::from_utf8(&file_table[file_offset + 0x20..name_end])
                .unwrap_or("")
                .to_string();

            let parent_path = dir_paths.get(&parent_offset).cloned().unwrap_or_default();
            let full_path = if parent_path.is_empty() {
                name
            } else {
                format!("{parent_path}/{name}")
            };

            files.insert(Self::normalize_path(&full_path), (data_offset, data_size));

            let padded_len = (name_length + 3) & !3;
            file_offset += 0x20 + padded_len;
        }

        Ok(Self {
            files,
            data_bytes: image.to_vec(),
            file_data_offset,
        })
    }

    /// Parses RomFS from an NCA section decrypted payload using its `NcaFsHeader`.
    /// Handles IVFC (Hierarchical Integrity) level descriptors.
    pub fn from_section_payload(payload: &[u8], fs_header: &NcaFsHeader) -> Result<Self, LoadError> {
        if fs_header.hash_type == NcaHashType::HierarchicalIntegrity {
            // Parse IVFC header in fs_header.hash_data (0xF8 bytes)
            if fs_header.hash_data.len() < 0x9C || &fs_header.hash_data[0..4] != IVFC_MAGIC {
                return Err(LoadError::Invalid("missing IVFC magic in RomFS hash data"));
            }

            // Find data level (Level 5 or highest active level)
            // Levels start at offset 0x0C, 24 bytes each (0x18)
            let mut data_level_offset = 0u64;
            let mut data_level_size = 0u64;

            for level_idx in (0..6).rev() {
                let off = 0x0C + level_idx * 0x18;
                let lvl_offset = u64::from_le_bytes(fs_header.hash_data[off..off + 8].try_into().unwrap());
                let lvl_size = u64::from_le_bytes(fs_header.hash_data[off + 8..off + 16].try_into().unwrap());
                if lvl_size > 0 {
                    data_level_offset = lvl_offset;
                    data_level_size = lvl_size;
                    break;
                }
            }

            if data_level_size == 0 || (payload.len() as u64) < data_level_offset + data_level_size {
                return Err(LoadError::Invalid("IVFC data level exceeds section payload bounds"));
            }

            let romfs_slice = &payload[data_level_offset as usize..(data_level_offset + data_level_size) as usize];
            Self::parse(romfs_slice)
        } else {
            // Standalone RomFS directly at payload offset 0
            Self::parse(payload)
        }
    }

    /// Normalizes path for lookup (strips leading slashes, "romfs:/", etc.)
    pub fn normalize_path(path: &str) -> String {
        let p = path.trim().replace('\\', "/");
        let p = p.strip_prefix("romfs:/").unwrap_or(&p);
        let p = p.strip_prefix('/').unwrap_or(p);
        p.to_string()
    }

    /// Returns the file bytes for a given path in the RomFS archive.
    pub fn get_file(&self, path: &str) -> Option<&[u8]> {
        let norm = Self::normalize_path(path);
        let &(offset, size) = self.files.get(&norm)?;
        let start = (self.file_data_offset + offset) as usize;
        let end = start.checked_add(size as usize)?;
        if end <= self.data_bytes.len() {
            Some(&self.data_bytes[start..end])
        } else {
            None
        }
    }

    /// Returns the file size for a given path.
    pub fn file_size(&self, path: &str) -> Option<u64> {
        let norm = Self::normalize_path(path);
        self.files.get(&norm).map(|&(_, size)| size)
    }

    /// Checks if a file exists in RomFS.
    pub fn contains_file(&self, path: &str) -> bool {
        let norm = Self::normalize_path(path);
        self.files.contains_key(&norm)
    }

    /// Returns a list of all normalized file paths contained in the archive.
    pub fn file_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.files.keys().cloned().collect();
        paths.sort();
        paths
    }
}

/// Helper builder for creating test RomFS images.
pub fn build_romfs_image(files: &[(&str, &[u8])]) -> Vec<u8> {
    // Collect unique directories and files
    // For simplicity, support root and 1-level subdirectories
    let mut dir_entries: Vec<u8> = Vec::new();
    let mut file_entries: Vec<u8> = Vec::new();
    let mut data_section: Vec<u8> = Vec::new();

    // Root directory entry at offset 0
    dir_entries.extend_from_slice(&0u32.to_le_bytes()); // parent (0 for root)
    dir_entries.extend_from_slice(&u32::MAX.to_le_bytes()); // sibling
    dir_entries.extend_from_slice(&u32::MAX.to_le_bytes()); // child dir
    dir_entries.extend_from_slice(&0u32.to_le_bytes()); // child file (first file at offset 0)
    dir_entries.extend_from_slice(&u32::MAX.to_le_bytes()); // next hash
    dir_entries.extend_from_slice(&0u32.to_le_bytes()); // name length 0

    let mut current_data_offset = 0u64;

    for (i, (name, content)) in files.iter().enumerate() {
        let sibling = if i + 1 < files.len() {
            // Next file offset in file_table
            // Current file entry size: 0x20 + padded name length
            let pad = (name.len() + 3) & !3;
            (file_entries.len() + 0x20 + pad) as u32
        } else {
            u32::MAX
        };

        let file_start = current_data_offset;
        let file_size = content.len() as u64;
        data_section.extend_from_slice(content);
        // Align data to 16 bytes
        let pad_bytes = ((content.len() + 15) & !15) - content.len();
        data_section.resize(data_section.len() + pad_bytes, 0);
        current_data_offset += (content.len() + pad_bytes) as u64;

        file_entries.extend_from_slice(&0u32.to_le_bytes()); // parent dir offset = 0
        file_entries.extend_from_slice(&sibling.to_le_bytes());
        file_entries.extend_from_slice(&file_start.to_le_bytes());
        file_entries.extend_from_slice(&file_size.to_le_bytes());
        file_entries.extend_from_slice(&u32::MAX.to_le_bytes()); // hash
        file_entries.extend_from_slice(&(name.len() as u32).to_le_bytes());
        file_entries.extend_from_slice(name.as_bytes());
        let pad = ((name.len() + 3) & !3) - name.len();
        file_entries.resize(file_entries.len() + pad, 0);
    }

    let dir_hash_size = 4usize;
    let file_hash_size = 4usize;
    let dir_table_size = dir_entries.len();
    let file_table_size = file_entries.len();

    let dir_hash_offset = ROMFS_HEADER_SIZE;
    let dir_table_offset = dir_hash_offset + dir_hash_size;
    let file_hash_offset = dir_table_offset + dir_table_size;
    let file_table_offset = file_hash_offset + file_hash_size;
    let file_data_offset = (file_table_offset + file_table_size + 15) & !15;

    let mut image = vec![0u8; file_data_offset];
    image[0x00..0x08].copy_from_slice(&(ROMFS_HEADER_SIZE as u64).to_le_bytes());
    image[0x08..0x10].copy_from_slice(&(dir_hash_offset as u64).to_le_bytes());
    image[0x10..0x18].copy_from_slice(&(dir_hash_size as u64).to_le_bytes());
    image[0x18..0x20].copy_from_slice(&(dir_table_offset as u64).to_le_bytes());
    image[0x20..0x28].copy_from_slice(&(dir_table_size as u64).to_le_bytes());
    image[0x28..0x30].copy_from_slice(&(file_hash_offset as u64).to_le_bytes());
    image[0x30..0x38].copy_from_slice(&(file_hash_size as u64).to_le_bytes());
    image[0x38..0x40].copy_from_slice(&(file_table_offset as u64).to_le_bytes());
    image[0x40..0x48].copy_from_slice(&(file_table_size as u64).to_le_bytes());
    image[0x48..0x50].copy_from_slice(&(file_data_offset as u64).to_le_bytes());

    // Copy tables
    image[dir_table_offset..dir_table_offset + dir_table_size].copy_from_slice(&dir_entries);
    image[file_table_offset..file_table_offset + file_table_size].copy_from_slice(&file_entries);

    // Append data
    image.extend_from_slice(&data_section);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_romfs_build_and_parse() {
        let sample_files = [
            ("main.lua", b"print('Hello Pokemon')".as_slice()),
            ("config.json", b"{\"fps\": 30, \"resolution\": \"720p\"}".as_slice()),
            ("title.txt", b"Pokemon Legends: Z-A".as_slice()),
        ];
        let image = build_romfs_image(&sample_files);
        let archive = RomFsArchive::parse(&image).expect("RomFS parsing should succeed");

        assert_eq!(archive.file_paths().len(), 3);
        assert!(archive.contains_file("main.lua"));
        assert!(archive.contains_file("romfs:/config.json"));
        assert!(archive.contains_file("/title.txt"));

        assert_eq!(archive.get_file("main.lua").unwrap(), b"print('Hello Pokemon')");
        assert_eq!(archive.file_size("title.txt").unwrap(), 20);
        assert_eq!(archive.get_file("title.txt").unwrap(), b"Pokemon Legends: Z-A");
    }

    #[test]
    fn test_romfs_invalid_header_rejection() {
        let mut bad_image = vec![0u8; 0x100];
        // Set bad header size
        bad_image[0] = 0x40;
        assert!(RomFsArchive::parse(&bad_image).is_err());
    }
}
