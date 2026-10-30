//! Original uncompressed NRO0 parsing and mapping. Module relocation is not implemented.

use emulator_memory::{AddressSpace, Mapping, MemoryError, Permissions};
use std::fmt;

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_NRO_FILE_SIZE: usize = 64 * 1024 * 1024;
pub const MAX_NRO_MEMORY: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    Invalid(&'static str),
    UnsupportedVersion(u32),
    UnsupportedFlags(u32),
    UnsupportedModuleHeader,
    LimitExceeded,
    AddressOverflow,
    Memory(MemoryError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid NRO: {reason}"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported NRO version {version}"),
            Self::UnsupportedFlags(flags) => write!(f, "unsupported NRO flags {flags:#x}"),
            Self::UnsupportedModuleHeader => {
                write!(f, "NRO module headers/relocations are not implemented")
            }
            Self::LimitExceeded => write!(f, "NRO file or memory limit exceeded"),
            Self::AddressOverflow => write!(f, "NRO guest address overflow"),
            Self::Memory(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LoadError {}
impl From<MemoryError> for LoadError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub offset: u64,
    pub file_size: usize,
    pub memory_size: usize,
    pub permissions: Permissions,
}

#[derive(Debug)]
pub struct NroImage {
    bytes: Vec<u8>,
    segments: [Segment; 3],
    build_id: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedImage {
    pub base: u64,
    pub entry: u64,
    pub end: u64,
    pub build_id: [u8; 32],
}

impl NroImage {
    pub fn parse(input: &[u8]) -> Result<Self, LoadError> {
        if input.len() < 0x80 {
            return Err(LoadError::Invalid("truncated header"));
        }
        if input.len() > MAX_NRO_FILE_SIZE {
            return Err(LoadError::LimitExceeded);
        }
        if &input[0x10..0x14] != b"NRO0" {
            return Err(LoadError::Invalid("missing NRO0 signature"));
        }
        let version = word(input, 0x14);
        if version != 0 {
            return Err(LoadError::UnsupportedVersion(version));
        }
        let flags = word(input, 0x1c);
        if flags != 0 {
            return Err(LoadError::UnsupportedFlags(flags));
        }
        if word(input, 4) != 0 {
            return Err(LoadError::UnsupportedModuleHeader);
        }
        if word(input, 0x3c) != 0 {
            return Err(LoadError::Invalid("additional headers are unsupported"));
        }
        let size = word(input, 0x18) as usize;
        if size < 0x80 || size > input.len() {
            return Err(LoadError::Invalid("declared image size exceeds file"));
        }
        let bss = word(input, 0x38) as usize;
        let mut segments = [Segment {
            offset: 0,
            file_size: 0,
            memory_size: 0,
            permissions: Permissions::NONE,
        }; 3];
        let permissions = [
            Permissions::READ_EXECUTE,
            Permissions::READ,
            Permissions::READ_WRITE,
        ];
        let mut previous_end = 0_u64;
        let mut total = 0_usize;
        for (index, segment) in segments.iter_mut().enumerate() {
            let offset = u64::from(word(input, 0x20 + index * 8));
            let file_size = word(input, 0x24 + index * 8) as usize;
            if offset % PAGE_SIZE != 0 || file_size as u64 % PAGE_SIZE != 0 {
                return Err(LoadError::Invalid("segments must be guest-page aligned"));
            }
            if index == 0 && (offset != 0 || file_size < 0x80) {
                return Err(LoadError::Invalid(
                    "text must contain the header at offset zero",
                ));
            }
            let end = offset
                .checked_add(file_size as u64)
                .ok_or(LoadError::AddressOverflow)?;
            if offset < previous_end || end > size as u64 {
                return Err(LoadError::Invalid("segment overlap or range outside image"));
            }
            let memory_size = file_size
                .checked_add(if index == 2 { bss } else { 0 })
                .ok_or(LoadError::LimitExceeded)?;
            total = total
                .checked_add(memory_size)
                .ok_or(LoadError::LimitExceeded)?;
            if total > MAX_NRO_MEMORY {
                return Err(LoadError::LimitExceeded);
            }
            *segment = Segment {
                offset,
                file_size,
                memory_size,
                permissions: permissions[index],
            };
            previous_end = end;
        }
        // Dynamic metadata is not interpreted by this initial static-image loader.
        if (0x60..0x80)
            .step_by(4)
            .any(|offset| word(input, offset) != 0)
        {
            return Err(LoadError::Invalid(
                "dynamic/embedded metadata is unsupported",
            ));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| LoadError::LimitExceeded)?;
        bytes.extend_from_slice(&input[..size]);
        let mut build_id = [0; 32];
        build_id.copy_from_slice(&input[0x40..0x60]);
        Ok(Self {
            bytes,
            segments,
            build_id,
        })
    }

    pub fn segments(&self) -> &[Segment; 3] {
        &self.segments
    }

    pub fn map_into(&self, memory: &mut AddressSpace, base: u64) -> Result<LoadedImage, LoadError> {
        if base % PAGE_SIZE != 0 {
            return Err(LoadError::Invalid("load base must be guest-page aligned"));
        }
        let mut mappings = Vec::new();
        let mut end = base;
        for segment in self.segments {
            let address = base
                .checked_add(segment.offset)
                .ok_or(LoadError::AddressOverflow)?;
            let segment_end = address
                .checked_add(segment.memory_size as u64)
                .ok_or(LoadError::AddressOverflow)?;
            end = end.max(segment_end);
            if segment.memory_size == 0 {
                continue;
            }
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(segment.memory_size)
                .map_err(|_| LoadError::LimitExceeded)?;
            let offset = segment.offset as usize;
            bytes.extend_from_slice(&self.bytes[offset..offset + segment.file_size]);
            bytes.resize(segment.memory_size, 0);
            mappings.push(Mapping {
                start: address,
                bytes,
                permissions: segment.permissions,
            });
        }
        memory.map_batch(mappings)?;
        Ok(LoadedImage {
            base,
            entry: base,
            end,
            build_id: self.build_id,
        })
    }
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated header field"),
    )
}
