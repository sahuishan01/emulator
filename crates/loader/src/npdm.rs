//! Original NPDM (Nintendo Program Descriptor Meta) parser.
//!
//! META header (0x80 bytes):
//! 0x00 magic "META", 0x0C flags (bit 0 = 64-bit, bits 1..=3 = address space
//! type), 0x0E main thread priority (u8), 0x0F default CPU core (u8),
//! 0x14 system resource size, 0x18 version, 0x1C main thread stack size,
//! 0x20 name (0x10), 0x30 product code (0x10), 0x70/0x74 ACI0 offset/size,
//! 0x78/0x7C ACID offset/size. The program ID lives in ACI0 at +0x10.

use crate::LoadError;

pub const NPDM_MAGIC: &[u8; 4] = b"META";
pub const NPDM_HEADER_SIZE: usize = 0x80;
pub const ACI0_MAGIC: &[u8; 4] = b"ACI0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpdmMeta {
    /// Program ID from ACI0; 0 when the descriptor has no ACI0 section.
    pub title_id: u64,
    pub main_thread_priority: i32,
    pub default_cpu_core: i32,
    pub main_thread_stack_size: u32,
    /// Kernel system resource size; nonzero enables virtual address memory
    /// (svcMapPhysicalMemory) in the guest SDK.
    pub system_resource_size: u32,
    pub version: u32,
    pub is_64bit: bool,
    pub address_space_type: u8,
    pub name: String,
    pub product_code: String,
}

fn ascii_field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("").trim().to_string()
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

impl NpdmMeta {
    pub fn parse(data: &[u8]) -> Result<Self, LoadError> {
        if data.len() < NPDM_HEADER_SIZE {
            return Err(LoadError::Invalid("NPDM binary smaller than header"));
        }
        if &data[0..4] != NPDM_MAGIC {
            return Err(LoadError::Invalid("invalid NPDM magic"));
        }
        let flags = data[0x0c];

        let aci_offset = u32_at(data, 0x70) as usize;
        let aci_size = u32_at(data, 0x74) as usize;
        let title_id = if aci_size == 0 {
            0
        } else {
            let aci = aci_offset
                .checked_add(aci_size)
                .and_then(|end| data.get(aci_offset..end))
                .ok_or(LoadError::Invalid("NPDM ACI0 section out of bounds"))?;
            if aci.len() < 0x18 || &aci[0..4] != ACI0_MAGIC {
                return Err(LoadError::Invalid("invalid NPDM ACI0 section"));
            }
            u64::from_le_bytes(aci[0x10..0x18].try_into().unwrap())
        };

        Ok(Self {
            title_id,
            main_thread_priority: i32::from(data[0x0e]),
            default_cpu_core: i32::from(data[0x0f]),
            main_thread_stack_size: u32_at(data, 0x1c),
            system_resource_size: u32_at(data, 0x14),
            version: u32_at(data, 0x18),
            is_64bit: flags & 1 != 0,
            address_space_type: (flags >> 1) & 7,
            name: ascii_field(&data[0x20..0x30]),
            product_code: ascii_field(&data[0x30..0x40]),
        })
    }
}
