//! Original bounded Maxwell command framing and synchronous DMA reference core.
//! Framing and execution are separate; neither submits to host Vulkan.
//! Wire fields follow NVIDIA's public MAXWELL_CHANNEL_GPFIFO_A interface.
use std::fmt;

mod address_space;
mod clear;
mod draw;
pub use draw::Draw;
mod graphics;
pub use graphics::{GraphicsBackend, GraphicsFrame};
mod methods;
pub use clear::{ColorClear, BGRA8_UNORM, MAXWELL_B, RGBA8_UNORM};
mod submission;
pub use address_space::{
    AddressSpaceId, GpuAddressSpace, GpuMemoryError, GpuMemoryRange, MappingId,
};
pub use methods::{
    ExecutionError, ExecutionLimits, ExecutionStats, LINEAR_DMA_LAUNCH, MAXWELL_DMA_COPY_A,
};
pub use submission::{
    ChannelProcessor, PreparedSubmission, QueueError, QueueLimits, QueueStats, Submission,
    SubmissionId, SubmissionQueue,
};

pub const MAX_PUSHBUFFER_WORDS: usize = 16 * 1024;
pub const MAX_METHOD_WRITES: usize = MAX_PUSHBUFFER_WORDS;
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

impl GpFifoEntry {
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != 8 {
            return Err(DecodeError::InvalidEntrySize);
        }
        let low = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
        let high = u32::from_le_bytes(bytes[4..].try_into().expect("four bytes"));
        let word_count = (high >> 10) & 0x1f_ffff;
        // Conditional fetch, reserved bit 1, kernel privilege, subroutine level,
        // sync-wait and zero-length control entries need semantics not yet owned.
        if low & 3 != 0 || high & 0x8000_0300 != 0 || word_count == 0 {
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
        Ok(Self {
            address,
            word_count,
        })
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
    if bytes.len() > MAX_PUSHBUFFER_WORDS * 4 {
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
