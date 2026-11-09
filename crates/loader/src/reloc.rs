//! Original dynamic ELF relocation processing for Nintendo Switch executables (NSO0 / NRO0).
//!
//! Nintendo Switch executables use standard ELF64 dynamic tags (.dynamic section)
//! and relocation entries with addends (Elf64_Rela).
//!
//! Key tags:
//! - DT_NULL = 0
//! - DT_RELA = 7 (address of relocation table)
//! - DT_RELASZ = 8 (total size of relocation table in bytes)
//! - DT_RELAENT = 9 (size of an individual relocation entry, typically 24 bytes)
//!
//! Key relocation types for AArch64:
//! - R_AARCH64_RELATIVE = 1027 (0x403): sets *target = base_address + addend

use crate::LoadError;
use emulator_memory::AddressSpace;

pub const DT_NULL: i64 = 0;
pub const DT_RELA: i64 = 7;
pub const DT_RELASZ: i64 = 8;
pub const DT_RELAENT: i64 = 9;

pub const R_AARCH64_RELATIVE: u32 = 1027;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Elf64Dyn {
    pub d_tag: i64,
    pub d_val: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Elf64Rela {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

impl Elf64Rela {
    #[inline]
    pub fn r_type(&self) -> u32 {
        (self.r_info & 0xffff_ffff) as u32
    }

    #[inline]
    pub fn r_sym(&self) -> u32 {
        (self.r_info >> 32) as u32
    }
}

/// Applies relocations from a .dynamic section at `dynamic_offset` relative to `base_address`.
pub fn apply_relocations(
    memory: &mut AddressSpace,
    base_address: u64,
    dynamic_offset: u64,
    max_dynamic_size: usize,
) -> Result<usize, LoadError> {
    let dyn_addr = base_address.checked_add(dynamic_offset).ok_or(LoadError::AddressOverflow)?;
    let mut rela_offset: Option<u64> = None;
    let mut rela_size: Option<u64> = None;
    let mut rela_ent: Option<u64> = None;

    let mut current_dyn = dyn_addr;
    let max_dyn_entries = max_dynamic_size / 16;

    for _ in 0..max_dyn_entries {
        let tag = memory.read_u64(current_dyn).map_err(LoadError::Memory)? as i64;
        let val = memory.read_u64(current_dyn + 8).map_err(LoadError::Memory)?;
        current_dyn += 16;

        if tag == DT_NULL {
            break;
        } else if tag == DT_RELA {
            rela_offset = Some(val);
        } else if tag == DT_RELASZ {
            rela_size = Some(val);
        } else if tag == DT_RELAENT {
            rela_ent = Some(val);
        }
    }

    let mut applied_count = 0;
    if let (Some(rela_off), Some(total_size)) = (rela_offset, rela_size) {
        let entry_size = rela_ent.unwrap_or(24);
        if entry_size != 24 {
            return Err(LoadError::Invalid("unsupported Elf64_Rela entry size"));
        }
        let count = total_size / entry_size;
        let rela_addr = base_address.checked_add(rela_off).ok_or(LoadError::AddressOverflow)?;

        for i in 0..count {
            let entry_addr = rela_addr.checked_add(i * 24).ok_or(LoadError::AddressOverflow)?;
            let r_offset = memory.read_u64(entry_addr).map_err(LoadError::Memory)?;
            let r_info = memory.read_u64(entry_addr + 8).map_err(LoadError::Memory)?;
            let r_addend = memory.read_u64(entry_addr + 16).map_err(LoadError::Memory)? as i64;

            let rela = Elf64Rela {
                r_offset,
                r_info,
                r_addend,
            };

            if rela.r_type() == R_AARCH64_RELATIVE {
                let target_addr = base_address.checked_add(rela.r_offset).ok_or(LoadError::AddressOverflow)?;
                let resolved_value = base_address.wrapping_add_signed(rela.r_addend);
                memory.write_u64(target_addr, resolved_value).map_err(LoadError::Memory)?;
                applied_count += 1;
            }
        }
    }

    Ok(applied_count)
}
