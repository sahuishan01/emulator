//! Original ExeFS (Executable Filesystem) loader.
//! Unpacks a PFS0 archive containing NSO modules and NPDM metadata into an executable program.

use crate::npdm::NpdmMeta;
use crate::nso::{NsoImage, NsoProgram};
use crate::pfs0::Pfs0Archive;
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
        let archive = Pfs0Archive::parse(archive_bytes)?;

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
            let filename = format!("{name}.nso");
            if let Some(nso_bytes) = archive.get_file(&filename) {
                let image = NsoImage::parse(nso_bytes)?;
                modules.push(ExeFsModule {
                    name: name.to_string(),
                    image,
                });
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
