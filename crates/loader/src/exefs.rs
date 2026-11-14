//! Original ExeFS (Executable Filesystem) loader.
//! Unpacks a PFS0 archive containing NSO modules and NPDM metadata into an executable program.

use crate::npdm::NpdmMeta;
use crate::nso::{NsoImage, NsoProgram};
use crate::pfs0::{Pfs0Archive, HFS0_MAGIC, PFS0_MAGIC};
use crate::LoadError;

#[derive(Debug)]
pub struct ExeFsModule {
    pub name: String,
    pub image: NsoImage,
}

#[derive(Debug)]
pub struct ExeFsProgram {
    pub npdm: Option<NpdmMeta>,
    pub modules: Vec<ExeFsModule>,
}

impl ExeFsProgram {
    /// Parses an ExeFS archive from raw bytes (PFS0 format containing main.npdm, rtld.nso, main.nso, subsdk*.nso).
    pub fn parse(archive_bytes: &[u8]) -> Result<Self, LoadError> {
        let archive_slice = if archive_bytes.starts_with(PFS0_MAGIC)
            || archive_bytes.starts_with(HFS0_MAGIC)
        {
            archive_bytes
        } else {
            // Search for PFS0 / HFS0 magic at 0x200-aligned offsets in the first 1 MiB
            let search_limit = archive_bytes.len().min(1024 * 1024);
            let mut found_offset = None;
            for off in (0x200..search_limit).step_by(0x200) {
                if off + 4 <= archive_bytes.len() {
                    let magic = &archive_bytes[off..off + 4];
                    if magic == PFS0_MAGIC || magic == HFS0_MAGIC {
                        if Pfs0Archive::parse(&archive_bytes[off..]).is_ok() {
                            found_offset = Some(off);
                            break;
                        }
                    }
                }
            }
            if let Some(off) = found_offset {
                &archive_bytes[off..]
            } else {
                archive_bytes
            }
        };

        let archive = Pfs0Archive::parse(archive_slice)?;

        let npdm = if let Some(meta_bytes) = archive.get_file("main.npdm") {
            Some(NpdmMeta::parse(meta_bytes)?)
        } else {
            None
        };

        let mut modules = Vec::new();

        // Standard load order for Switch executables:
        // 1. rtld (Runtime Linker, if present)
        // 2. main (Main application binary)
        // 3. subsdk0 .. subsdk9 (SDK and middleware dynamic libraries)
        // 4. sdk (SDK binary)
        let candidates = [
            "rtld", "main", "subsdk0", "subsdk1", "subsdk2", "subsdk3", "subsdk4", "subsdk5",
            "subsdk6", "subsdk7", "subsdk8", "subsdk9", "sdk",
        ];

        for name in candidates {
            let nso_bytes = archive
                .get_file(name)
                .or_else(|| archive.get_file(&format!("{name}.nso")));
            if let Some(bytes) = nso_bytes {
                let image = NsoImage::parse(bytes)?;
                modules.push(ExeFsModule {
                    name: name.to_string(),
                    image,
                });
            }
        }

        // Fallback: scan all files in the archive for NSO0 magic (excluding metadata and logo files)
        if modules.is_empty() {
            for file in archive.files() {
                let lower = file.name.to_ascii_lowercase();
                if lower == "main.npdm" || lower.ends_with(".png") || lower.ends_with(".gif") {
                    continue;
                }
                if let Some(bytes) = archive.get_file(&file.name) {
                    if bytes.len() >= 4 && &bytes[..4] == b"NSO0" {
                        if let Ok(image) = NsoImage::parse(bytes) {
                            modules.push(ExeFsModule {
                                name: file.name.clone(),
                                image,
                            });
                        }
                    }
                }
            }
        }

        if modules.is_empty() {
            return Err(LoadError::Invalid("ExeFS contains no valid NSO modules"));
        }

        Ok(Self { npdm, modules })
    }

    /// Loads all modules of the ExeFS program into memory using an NsoProgram.
    pub fn load_into(
        &self,
        program: &mut NsoProgram,
        memory: &mut emulator_memory::AddressSpace,
        start_base: u64,
    ) -> Result<(), LoadError> {
        let mut base = start_base;
        for module in &self.modules {
            let loaded = program.load_module(&module.name, &module.image, memory, base)?;
            base = (loaded.end + crate::PAGE_SIZE - 1) & !(crate::PAGE_SIZE - 1);
        }
        Ok(())
    }
}
