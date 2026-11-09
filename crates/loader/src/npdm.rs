//! Original NPDM (Nintendo Program Descriptor Meta) parser.
//! Contains Title ID, product code, main thread priority, default CPU core,
//! and Service Access Control (SAC) capabilities.

use crate::LoadError;

pub const NPDM_MAGIC: &[u8; 4] = b"META";
pub const NPDM_HEADER_SIZE: usize = 0x80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpdmMeta {
    pub title_id: u64,
    pub main_thread_priority: i32,
    pub default_cpu_core: i32,
    pub main_thread_stack_size: u32,
    pub product_code: String,
}

impl NpdmMeta {
    pub fn parse(data: &[u8]) -> Result<Self, LoadError> {
        if data.len() < NPDM_HEADER_SIZE {
            return Err(LoadError::Invalid("NPDM binary smaller than header"));
        }
        if &data[0..4] != NPDM_MAGIC {
            return Err(LoadError::Invalid("invalid NPDM magic"));
        }

        // Product code at 0x10..0x20 (16 bytes ASCII null-padded)
        let product_bytes = &data[0x10..0x20];
        let null_pos = product_bytes.iter().position(|&b| b == 0).unwrap_or(16);
        let product_code = std::str::from_utf8(&product_bytes[..null_pos])
            .unwrap_or("")
            .trim()
            .to_string();

        let title_id = u64::from_le_bytes(data[0x20..0x28].try_into().unwrap());
        let main_thread_priority = i32::from_le_bytes(data[0x28..0x2c].try_into().unwrap());
        let default_cpu_core = i32::from_le_bytes(data[0x2c..0x30].try_into().unwrap());
        let main_thread_stack_size = u32::from_le_bytes(data[0x34..0x38].try_into().unwrap());

        Ok(Self {
            title_id,
            main_thread_priority,
            default_cpu_core,
            main_thread_stack_size,
            product_code,
        })
    }
}
