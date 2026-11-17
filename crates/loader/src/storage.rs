//! Read-only, random-access storages backing fsp-srv `IStorage`.
//!
//! Commercial RomFS sections are gigabytes, so they are never decrypted up front:
//! [`SectionStorage`] keeps a shared handle to the container bytes and decrypts
//! exactly the requested range with AES-CTR on every read.

use crate::nca::{make_section_ctr, NcaDecryptedHeader, NcaEncryptionType, NcaFsHeader, NcaFsType, NcaHashType};
use crate::romfs::RomFsArchive;
use crate::LoadError;
use emulator_crypto::aes128::aes128_ctr_xor;
use emulator_crypto::keys::KeyFile;
use std::fmt;
use std::rc::Rc;

/// RomFS header size; the first u64 of every RomFS image.
const ROMFS_HEADER_SIZE: u64 = 0x50;
/// IVFC magic at the start of a HierarchicalIntegrity hash header.
const IVFC_MAGIC: &[u8; 4] = b"IVFC";

/// Shared, immutable container bytes (heap buffer or memory-mapped file).
pub type SharedBytes = Rc<dyn AsRef<[u8]>>;

pub trait ReadOnlyStorage: fmt::Debug {
    fn size(&self) -> u64;
    /// Fills `output` from `offset`. Out-of-range reads fail without partial output.
    fn read(&self, offset: u64, output: &mut [u8]) -> Result<(), LoadError>;
}

impl ReadOnlyStorage for RomFsArchive {
    fn size(&self) -> u64 {
        self.image().len() as u64
    }

    fn read(&self, offset: u64, output: &mut [u8]) -> Result<(), LoadError> {
        let start = usize::try_from(offset).map_err(|_| LoadError::Invalid("storage read out of range"))?;
        let bytes = start
            .checked_add(output.len())
            .and_then(|end| self.image().get(start..end))
            .ok_or(LoadError::Invalid("storage read out of range"))?;
        output.copy_from_slice(bytes);
        Ok(())
    }
}

/// A byte range of a shared container, optionally AES-CTR encrypted with the
/// NCA section counter (whose block index is the absolute offset in the NCA).
pub struct SectionStorage {
    container: SharedBytes,
    /// Offset of the NCA inside the container (an NSP holds several NCAs).
    nca_offset: u64,
    /// Offset of this storage's first byte inside the NCA.
    start: u64,
    length: u64,
    cipher: Option<([u8; 16], [u8; 8])>,
}

impl fmt::Debug for SectionStorage {
    // Never print key material.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SectionStorage")
            .field("nca_offset", &self.nca_offset)
            .field("start", &self.start)
            .field("length", &self.length)
            .field("encrypted", &self.cipher.is_some())
            .finish()
    }
}

impl ReadOnlyStorage for SectionStorage {
    fn size(&self) -> u64 {
        self.length
    }

    fn read(&self, offset: u64, output: &mut [u8]) -> Result<(), LoadError> {
        let out_of_range = LoadError::Invalid("storage read out of range");
        let end = offset.checked_add(output.len() as u64).ok_or(out_of_range.clone())?;
        if end > self.length {
            return Err(out_of_range);
        }
        if output.is_empty() {
            return Ok(());
        }
        let nca_position = self.start + offset;
        let container: &[u8] = (*self.container).as_ref();
        let Some((key, section_ctr)) = &self.cipher else {
            let from = (self.nca_offset + nca_position) as usize;
            output.copy_from_slice(&container[from..from + output.len()]);
            return Ok(());
        };
        // CTR works on 16-byte blocks: decrypt the covering aligned span.
        let aligned = nca_position & !15;
        let skip = (nca_position - aligned) as usize;
        let span = (skip + output.len()).next_multiple_of(16);
        let from = (self.nca_offset + aligned) as usize;
        let available = container.len().saturating_sub(from).min(span);
        let mut block = vec![0u8; span];
        block[..available].copy_from_slice(&container[from..from + available]);
        aes128_ctr_xor(key, make_section_ctr(section_ctr, aligned), &mut block)
            .map_err(|_| LoadError::Invalid("NCA3 CTR decryption failed"))?;
        output.copy_from_slice(&block[skip..skip + output.len()]);
        Ok(())
    }
}

/// Candidate offsets (within the section) of the RomFS image for this hash type.
fn romfs_candidates(fs_header: &NcaFsHeader, section_length: u64) -> Vec<(u64, u64)> {
    let mut candidates = Vec::new();
    let (data_offset, data_size) = fs_header.data_region(section_length);
    if data_size > 0 {
        candidates.push((data_offset, data_size));
    }
    if matches!(
        fs_header.hash_type,
        NcaHashType::HierarchicalIntegrity | NcaHashType::HierarchicalIntegritySha3
    ) && &fs_header.hash_data[0..4] == IVFC_MAGIC
    {
        // Highest populated IVFC level is the data level (level table at 0x10, 0x18 each).
        for level in (0..6).rev() {
            let at = 0x10 + level * 0x18;
            let offset = u64::from_le_bytes(fs_header.hash_data[at..at + 8].try_into().unwrap());
            let size = u64::from_le_bytes(fs_header.hash_data[at + 8..at + 16].try_into().unwrap());
            if size > 0 {
                candidates.push((offset, size));
                break;
            }
        }
    }
    candidates.push((0, section_length));
    candidates
}

impl NcaDecryptedHeader {
    /// Opens the RomFS section of the NCA at `nca_offset` inside `container` as a
    /// lazily decrypted storage. The key is chosen by decrypting the RomFS header.
    pub fn open_romfs_storage(
        &self,
        shared: SharedBytes,
        nca_offset: u64,
        keys: &KeyFile,
    ) -> Result<SectionStorage, LoadError> {
        let container: &[u8] = (*shared).as_ref();
        for (index, section) in self.metadata.sections.iter().enumerate() {
            let Some(section) = section else { continue };
            let Ok(fs_header) = NcaFsHeader::parse(&self.fs_headers[index]) else {
                continue;
            };
            if fs_header.fs_type != NcaFsType::RomFs {
                continue;
            }
            if nca_offset.checked_add(section.end).is_none_or(|end| end > container.len() as u64) {
                return Err(LoadError::Invalid("NCA3 archive is shorter than section"));
            }
            let section_length = section.end - section.start;
            let header_at = |nca_position: u64, cipher: Option<&[u8; 16]>| -> Option<bool> {
                let from = (nca_offset + (nca_position & !15)) as usize;
                let mut block: [u8; 32] = container.get(from..from + 32)?.try_into().ok()?;
                if let Some(key) = cipher {
                    aes128_ctr_xor(key, make_section_ctr(&fs_header.section_ctr, nca_position & !15), &mut block)
                        .ok()?;
                }
                let skip = (nca_position & 15) as usize;
                Some(block[skip..skip + 8] == ROMFS_HEADER_SIZE.to_le_bytes())
            };
            let plain = fs_header.encryption_type == NcaEncryptionType::None
                || (!self.metadata.has_rights_id() && self.metadata.encrypted_key_area.iter().all(|k| *k == [0; 16]));
            let keys_to_try: Vec<Option<[u8; 16]>> = if plain {
                vec![None]
            } else {
                let mut list: Vec<Option<[u8; 16]>> =
                    self.candidate_section_keys(index, keys).into_iter().map(Some).collect();
                list.push(None); // sections stored decrypted in place
                list
            };
            for (offset, size) in romfs_candidates(&fs_header, section_length) {
                if offset.checked_add(size).is_none_or(|end| end > section_length) {
                    continue;
                }
                for key in &keys_to_try {
                    if header_at(section.start + offset, key.as_ref()) == Some(true) {
                        return Ok(SectionStorage {
                            container: Rc::clone(&shared),
                            nca_offset,
                            start: section.start + offset,
                            length: size,
                            cipher: key.map(|key| (key, fs_header.section_ctr)),
                        });
                    }
                }
            }
            return Err(LoadError::Invalid(
                "NCA3 RomFS: no key or IVFC level yields a RomFS header",
            ));
        }
        Err(LoadError::Invalid("NCA3 archive does not contain a RomFS section"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage(cipher: Option<([u8; 16], [u8; 8])>) -> (SectionStorage, Vec<u8>) {
        // Container: 0x100 bytes of padding, then an "NCA" whose section data
        // starts at NCA offset 0x40.
        let plain: Vec<u8> = (0..0x200u32).map(|i| (i * 7 + 3) as u8).collect();
        let mut container = vec![0xaa; 0x100 + 0x40];
        let mut section = plain.clone();
        if let Some((key, ctr)) = &cipher {
            aes128_ctr_xor(key, make_section_ctr(ctr, 0x40), &mut section).unwrap();
        }
        container.extend_from_slice(&section);
        let storage = SectionStorage {
            container: Rc::new(container),
            nca_offset: 0x100,
            start: 0x40,
            length: plain.len() as u64,
            cipher,
        };
        (storage, plain)
    }

    #[test]
    fn encrypted_reads_match_plaintext_at_any_alignment() {
        let (storage, plain) = storage(Some(([0x42; 16], [1, 2, 3, 4, 5, 6, 7, 8])));
        for (offset, length) in [(0, 16), (3, 29), (15, 1), (16, 0x100), (0x1f0, 0x10), (5, 0x1fb)] {
            let mut out = vec![0; length];
            storage.read(offset as u64, &mut out).unwrap();
            assert_eq!(out, plain[offset..offset + length], "offset {offset} len {length}");
        }
        assert!(!format!("{storage:?}").contains("66"), "debug output must not leak the key");
    }

    #[test]
    fn plain_reads_and_range_rejections() {
        let (storage, plain) = storage(None);
        let mut out = vec![0; 9];
        storage.read(0x11, &mut out).unwrap();
        assert_eq!(out, plain[0x11..0x1a]);
        let mut sentinel = vec![0x5a; 4];
        assert!(storage.read(0x1fd, &mut sentinel).is_err());
        assert_eq!(sentinel, [0x5a; 4], "failed reads leave the output untouched");
        assert!(storage.read(u64::MAX, &mut sentinel).is_err());
        assert!(storage.read(0x200, &mut []).is_ok());
    }
}
