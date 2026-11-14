//! Clean-room Nintendo Submission Package (NSP / PFS0) parser and loader.
//!
//! Nintendo Switch submission packages (NSP) are PFS0 archives containing:
//! - One or more Nintendo Content Archives (`.nca`)
//! - Content Metadata NCA (`.cnmt.nca`)
//! - Optional ticket files (`.tik`) containing encrypted title keys
//! - Optional certificates (`.cert`) and metadata XML (`.cnmt.xml`)
//!
//! This module provides full NSP inspection, ticket import with title key
//! unwrapping, CNMT extraction, and Program NCA identification.

use crate::cnmt::Cnmt;
use crate::nca::{decrypt_nca3_header, NcaContentType, NcaDecryptedHeader, NcaFsHeader, NcaFsType};
use crate::pfs0::Pfs0Archive;
use crate::ticket::Ticket;
use crate::LoadError;
use emulator_crypto::keys::KeyFile;

#[derive(Clone, Debug)]
pub struct NspArchive<'a> {
    pfs0: Pfs0Archive<'a>,
}

impl<'a> NspArchive<'a> {
    /// Parse an NSP package image.
    pub fn parse(data: &'a [u8]) -> Result<Self, LoadError> {
        let pfs0 = Pfs0Archive::parse(data)?;
        Ok(Self { pfs0 })
    }

    /// Access the underlying PFS0 partition archive.
    pub fn pfs0(&self) -> &Pfs0Archive<'a> {
        &self.pfs0
    }

    /// List all raw files present inside the NSP archive.
    pub fn filenames(&self) -> Vec<&str> {
        self.pfs0.files().iter().map(|f| f.name.as_str()).collect()
    }

    /// Retrieve raw bytes of a file inside the NSP by exact name.
    pub fn get_file(&self, name: &str) -> Option<&'a [u8]> {
        self.pfs0.get_file(name)
    }

    /// Parse and return all Ticket (`.tik`) records present in the NSP.
    pub fn parse_tickets(&self) -> Vec<Ticket> {
        let mut tickets = Vec::new();
        for file in self.pfs0.files() {
            if file.name.ends_with(".tik") {
                if let Some(bytes) = self.pfs0.get_file(&file.name) {
                    if let Ok(ticket) = Ticket::parse(bytes) {
                        tickets.push(ticket);
                    }
                }
            }
        }
        tickets
    }

    /// Import all common tickets found in the NSP into the provided `KeyFile`.
    /// Decrypts each title key using `titlekek` matching the ticket's master key revision.
    /// Returns the number of successfully imported title keys.
    pub fn import_tickets(&self, keys: &mut KeyFile) -> usize {
        let mut count = 0;
        for ticket in self.parse_tickets() {
            if ticket.import_into(keys).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Locate and parse the CNMT metadata from the `.cnmt.nca` archive.
    pub fn parse_cnmt(&self, keys: &KeyFile) -> Result<Option<Cnmt>, LoadError> {
        // Look for .cnmt.nca file or scan NCAs for Meta content type
        let meta_nca_data = if let Some(meta_file) = self
            .pfs0
            .files()
            .iter()
            .find(|f| f.name.ends_with(".cnmt.nca"))
        {
            self.pfs0.get_file(&meta_file.name)
        } else {
            // Scan all .nca files for Meta content type
            let mut found = None;
            for file in self.pfs0.files() {
                if file.name.ends_with(".nca") {
                    if let Some(data) = self.pfs0.get_file(&file.name) {
                        if let Ok(hdr) = decrypt_nca3_header(data, keys) {
                            if hdr.metadata.content_type == NcaContentType::Meta {
                                found = Some(data);
                                break;
                            }
                        }
                    }
                }
            }
            found
        };

        let Some(meta_nca) = meta_nca_data else {
            return Ok(None);
        };

        let header = decrypt_nca3_header(meta_nca, keys)?;

        // Find Section 0 (PartitionFS containing the CNMT)
        for (i, sec_opt) in header.metadata.sections.iter().enumerate() {
            if sec_opt.is_some() {
                let fs_header = NcaFsHeader::parse(&header.fs_headers[i])?;
                if fs_header.fs_type == NcaFsType::PartitionFs {
                    let decrypted_pfs = header.decrypt_section(meta_nca, i, keys)?;
                    let pfs = Pfs0Archive::parse(&decrypted_pfs)?;
                    for file in pfs.files() {
                        if file.name.ends_with(".cnmt") {
                            if let Some(cnmt_bytes) = pfs.get_file(&file.name) {
                                let cnmt = Cnmt::parse(cnmt_bytes)?;
                                return Ok(Some(cnmt));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Locate the raw bytes of the Program NCA inside the NSP.
    ///
    /// Checks CNMT records first; if CNMT is missing or doesn't match,
    /// falls back to decrypting NCA headers to find `NcaContentType::Program`.
    pub fn find_program_nca(&self, keys: &KeyFile) -> Result<&'a [u8], LoadError> {
        // First try via CNMT
        if let Ok(Some(cnmt)) = self.parse_cnmt(keys) {
            if let Some(prog_record) = cnmt.find_program_record() {
                let filename = prog_record.nca_filename();
                if let Some(data) = self.pfs0.get_file(&filename) {
                    return Ok(data);
                }
            }
        }

        // Fallback: scan all .nca files in the NSP
        for file in self.pfs0.files() {
            if file.name.ends_with(".nca") && !file.name.ends_with(".cnmt.nca") {
                if let Some(data) = self.pfs0.get_file(&file.name) {
                    if let Ok(header) = decrypt_nca3_header(data, keys) {
                        if header.metadata.content_type == NcaContentType::Program {
                            return Ok(data);
                        }
                    }
                }
            }
        }

        Err(LoadError::Invalid("NSP archive contains no valid Program NCA"))
    }

    /// Locate the raw bytes of the Control NCA inside the NSP, if present.
    pub fn find_control_nca(&self, keys: &KeyFile) -> Result<Option<&'a [u8]>, LoadError> {
        // Try via CNMT
        if let Ok(Some(cnmt)) = self.parse_cnmt(keys) {
            if let Some(ctrl_record) = cnmt.find_control_record() {
                let filename = ctrl_record.nca_filename();
                if let Some(data) = self.pfs0.get_file(&filename) {
                    return Ok(Some(data));
                }
            }
        }

        // Fallback: scan .nca files
        for file in self.pfs0.files() {
            if file.name.ends_with(".nca") && !file.name.ends_with(".cnmt.nca") {
                if let Some(data) = self.pfs0.get_file(&file.name) {
                    if let Ok(header) = decrypt_nca3_header(data, keys) {
                        if header.metadata.content_type == NcaContentType::Control {
                            return Ok(Some(data));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Import tickets, locate the Program NCA, decrypt its header, and verify hashes.
    pub fn open_program(
        &self,
        keys: &mut KeyFile,
    ) -> Result<(NcaDecryptedHeader, &'a [u8]), LoadError> {
        // 1. Import tickets so any title keys are available
        self.import_tickets(keys);

        // 2. Find Program NCA
        let prog_nca = self.find_program_nca(keys)?;

        // 3. Decrypt header
        let header = decrypt_nca3_header(prog_nca, keys)?;

        // 4. Verify integrity
        header.verify_fs_header_hashes()?;

        Ok((header, prog_nca))
    }
}
