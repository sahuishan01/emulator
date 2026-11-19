//! Original bounded Maxwell command framing and synchronous DMA reference core.
//! Framing and execution are separate; neither submits to host Vulkan.
//! Wire fields follow NVIDIA's public MAXWELL_CHANNEL_GPFIFO_A interface.
use std::fmt;

mod address_space;
mod clear;
mod depth;
mod draw;
pub use depth::{DepthClear, DepthState};
pub use draw::{Draw, IndexBuffer};
mod sampled_texture;
pub use sampled_texture::SampledTexture;
mod graphics;
pub use graphics::{GraphicsBackend, GraphicsFrame};
mod methods;
pub use clear::{ColorClear, BGRA8_UNORM, MAXWELL_B, RGBA8_UNORM};
mod submission;
pub use address_space::{
    AddressSpaceId, GpuAddressSpace, GpuMemoryError, GpuMemoryRange, MappingId,
};
pub use methods::{
    class_allowed, ExecutionError, ExecutionLimits, ExecutionStats, FERMI_TWOD_A,
    KEPLER_INLINE_TO_MEMORY_B, LINEAR_DMA_LAUNCH, MAXWELL_COMPUTE_B, MAXWELL_DMA_COPY_A,
};
pub use submission::{
    ChannelProcessor, PreparedSubmission, QueueError, QueueLimits, QueueStats, Submission,
    SubmissionId, SubmissionQueue, CENSUS_SEPARATOR,
};

pub const MAX_PUSHBUFFER_WORDS: usize = 16 * 1024;
/// Words captured by one GPFIFO submit across all its entries (4 MiB). NVN
/// submits hundreds of segments at once (Z-A: 587 entries, 19.8K words).
pub const MAX_SUBMIT_WORDS: usize = 1 << 20;
pub const MAX_METHOD_WRITES: usize = MAX_SUBMIT_WORDS;
const GPU_ADDRESS_LIMIT: u64 = 1 << 40;

fn allocate_identity(counter: &std::sync::atomic::AtomicU64) -> Option<u64> {
    use std::sync::atomic::Ordering::Relaxed;
    let mut id = counter.load(Relaxed);
    loop {
        let next = id.checked_add(1)?;
        match counter.compare_exchange_weak(id, next, Relaxed, Relaxed) {
            Ok(_) => return Some(id),
            Err(actual) => id = actual,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuVa(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpFifoEntry {
    pub address: GpuVa,
    pub word_count: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodWrite {
    pub subchannel: u8,
    pub method_word: u16,
    pub data: u32,
}
impl MethodWrite {
    pub fn method_byte_offset(self) -> u32 {
        u32::from(self.method_word) * 4
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    InvalidEntrySize,
    UnsupportedEntry,
    AddressOverflow,
    TooLarge,
    PartialWord,
    LengthMismatch,
    InvalidBudget,
    WriteBudget { word_offset: usize },
    UnsupportedPacket { word_offset: usize, header: u32 },
    TruncatedPacket { word_offset: usize },
    MethodOverflow { word_offset: usize },
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GPU command decode failed: {self:?}")
    }
}
impl std::error::Error for DecodeError {}

/// One GPFIFO entry: a pushbuffer fetch, or a zero-length control entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpEntry {
    /// Main-level segment: a packet may continue into the next main segment.
    Fetch(GpFifoEntry),
    /// Subroutine-level segment: decoded from fresh packet state, after
    /// which the main segment's state resumes, so it must hold complete
    /// packets.
    Subroutine(GpFifoEntry),
    /// Zero-length entry with control opcode NOP: nothing to fetch.
    ControlNop,
}
impl GpEntry {
    /// Entry word 0: fetch mode (bits 0-1), address bits 2-31 (or the control
    /// opcode in bits 0-7 when the length is zero). Word 1: address bits
    /// 32-39 (0-7), privilege (8), subroutine level (9), length in words
    /// (10-30), sync (31). SYNC=wait only orders the fetch after earlier
    /// pushbuffer data is consumed; submissions execute in order to
    /// completion here, so it needs no further action.
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != 8 {
            return Err(DecodeError::InvalidEntrySize);
        }
        let low = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
        let high = u32::from_le_bytes(bytes[4..].try_into().expect("four bytes"));
        let word_count = (high >> 10) & 0x1f_ffff;
        // Kernel privilege needs semantics not yet owned.
        if high & 0x100 != 0 {
            return Err(DecodeError::UnsupportedEntry);
        }
        let subroutine = high & 0x200 != 0;
        if word_count == 0 {
            // Control entries: only NOP (opcode 0, no address, main level)
            // is modelled; ILLEGAL and the CRC checks fail explicitly.
            return if low == 0 && high & 0x2ff == 0 {
                Ok(Self::ControlNop)
            } else {
                Err(DecodeError::UnsupportedEntry)
            };
        }
        // Conditional fetch and reserved fetch mode bit 1.
        if low & 3 != 0 {
            return Err(DecodeError::UnsupportedEntry);
        }
        if word_count as usize > MAX_PUSHBUFFER_WORDS {
            return Err(DecodeError::TooLarge);
        }
        let address = GpuVa(u64::from(low) | (u64::from(high & 0xff) << 32));
        let end = address.0 + u64::from(word_count) * 4;
        if end > GPU_ADDRESS_LIMIT {
            return Err(DecodeError::AddressOverflow);
        }
        let entry = GpFifoEntry {
            address,
            word_count,
        };
        Ok(if subroutine {
            Self::Subroutine(entry)
        } else {
            Self::Fetch(entry)
        })
    }
}
impl GpFifoEntry {
    /// A main-level pushbuffer fetch entry; subroutine and control entries
    /// are `UnsupportedEntry` here (see [`GpEntry::parse`]).
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        match GpEntry::parse(bytes)? {
            GpEntry::Fetch(entry) => Ok(entry),
            GpEntry::Subroutine(_) | GpEntry::ControlNop => Err(DecodeError::UnsupportedEntry),
        }
    }
    /// The caller must separately resolve/read GPU virtual memory. This only
    /// validates the captured bytes' declared length and command framing.
    pub fn decode(
        self,
        bytes: &[u8],
        write_budget: usize,
    ) -> Result<Vec<MethodWrite>, DecodeError> {
        if self.word_count == 0 {
            return Err(DecodeError::UnsupportedEntry);
        }
        if self.word_count as usize > MAX_PUSHBUFFER_WORDS {
            return Err(DecodeError::TooLarge);
        }
        if self.address.0 % 4 != 0
            || self.address.0 >= GPU_ADDRESS_LIMIT
            || self
                .address
                .0
                .checked_add(u64::from(self.word_count) * 4)
                .is_none_or(|end| end > GPU_ADDRESS_LIMIT)
        {
            return Err(DecodeError::AddressOverflow);
        }
        if bytes.len() != self.word_count as usize * 4 {
            return Err(DecodeError::LengthMismatch);
        }
        decode_pushbuffer(bytes, write_budget)
    }
}

/// Parse complete captured packets atomically. Legacy/control forms remain
/// explicit errors. No partial output is observable on any failure.
pub fn decode_pushbuffer(
    bytes: &[u8],
    write_budget: usize,
) -> Result<Vec<MethodWrite>, DecodeError> {
    if bytes.len() > MAX_SUBMIT_WORDS * 4 {
        return Err(DecodeError::TooLarge);
    }
    if bytes.len() % 4 != 0 {
        return Err(DecodeError::PartialWord);
    }
    if write_budget > MAX_METHOD_WRITES {
        return Err(DecodeError::InvalidBudget);
    }
    let word =
        |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("validated word"));
    let mut at = 0;
    let mut output = Vec::new();
    let count = bytes.len() / 4;
    while at < count {
        let start = at;
        let header = word(at);
        at += 1;
        if header == 0 {
            continue;
        } // Exact documented DMA no-op, not opcode-0 catchall.
        let opcode = header >> 29;
        if !matches!(opcode, 1 | 3 | 4 | 5) || header & 0x1000 != 0 {
            return Err(DecodeError::UnsupportedPacket {
                word_offset: start,
                header,
            });
        }
        let method = header & 0xfff;
        let subchannel = ((header >> 13) & 7) as u8;
        let field = ((header >> 16) & 0x1fff) as usize;
        let writes = if opcode == 4 { 1 } else { field };
        if writes > write_budget.saturating_sub(output.len()) {
            return Err(DecodeError::WriteBudget { word_offset: start });
        }
        if opcode != 4 && writes > count - at {
            return Err(DecodeError::TruncatedPacket { word_offset: start });
        }
        let increment = match opcode {
            1 => writes.saturating_sub(1),
            5 if writes > 1 => 1,
            _ => 0,
        };
        if method as usize + increment > 0xfff {
            return Err(DecodeError::MethodOverflow { word_offset: start });
        }
        for index in 0..writes {
            let offset = match opcode {
                1 => index,
                5 if index > 0 => 1,
                _ => 0,
            };
            output.push(MethodWrite {
                subchannel,
                method_word: (method + offset as u32) as u16,
                data: if opcode == 4 {
                    field as u32
                } else {
                    word(at + index)
                },
            });
        }
        if opcode != 4 {
            at += writes;
        }
    }
    Ok(output)
}
