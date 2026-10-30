//! Original bounded HIPC/CMIF v0 subset. Domains, buffers and incoming handles
//! are deliberately rejected until their transfer semantics are implemented.

use std::fmt;

pub const IPC_BUFFER_SIZE: usize = 0x100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcError {
    Truncated,
    TooLarge,
    UnsupportedLayout,
    UnsupportedType(u16),
    InvalidMagic,
    UnsupportedVersion(u32),
    UnsupportedContext,
    UnalignedPayload,
}
impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported or malformed IPC message: {self:?}")
    }
}
impl std::error::Error for IpcError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandType {
    Request,
    Control,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: CommandType,
    pub command_id: u32,
    /// The kernel replaces this untrusted wire value with the sender's identity.
    pub pid: Option<u64>,
    pub data: Vec<u8>,
}

fn word(input: &[u8], offset: usize) -> Result<u32, IpcError> {
    let bytes = input.get(offset..offset + 4).ok_or(IpcError::Truncated)?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
}

pub fn parse_request(input: &[u8]) -> Result<Request, IpcError> {
    if input.len() > IPC_BUFFER_SIZE {
        return Err(IpcError::TooLarge);
    }
    let first = word(input, 0)?;
    let second = word(input, 4)?;
    if first >> 16 != 0 || second & 0x7fff_fc00 != 0 {
        return Err(IpcError::UnsupportedLayout);
    }
    let kind = match first as u16 {
        4 => CommandType::Request,
        5 => CommandType::Control,
        other => return Err(IpcError::UnsupportedType(other)),
    };
    let mut raw_start = 8;
    let pid = if second >> 31 != 0 {
        let special = word(input, raw_start)?;
        raw_start += 4;
        if special & !1 != 0 {
            return Err(IpcError::UnsupportedLayout);
        }
        if special == 0 {
            None
        } else {
            let bytes = input
                .get(raw_start..raw_start + 8)
                .ok_or(IpcError::Truncated)?;
            raw_start += 8;
            Some(u64::from_le_bytes(bytes.try_into().expect("eight bytes")))
        }
    } else {
        None
    };
    let raw_size = ((second & 0x3ff) as usize) * 4;
    if raw_size < 32 {
        return Err(IpcError::Truncated);
    }
    let raw_end = raw_start + raw_size;
    if raw_end > IPC_BUFFER_SIZE {
        return Err(IpcError::TooLarge);
    }
    if raw_end > input.len() {
        return Err(IpcError::Truncated);
    }
    let cmif = (raw_start + 15) & !15;
    if word(input, cmif)? != u32::from_le_bytes(*b"SFCI") {
        return Err(IpcError::InvalidMagic);
    }
    let version = word(input, cmif + 4)?;
    if version != 0 {
        return Err(IpcError::UnsupportedVersion(version));
    }
    if word(input, cmif + 12)? != 0 {
        return Err(IpcError::UnsupportedContext);
    }
    // CMIF reserves 16 bytes total alignment padding, independent of where
    // its raw data starts, plus its own 16-byte input header.
    let payload_start = cmif + 16;
    let payload_end = payload_start + raw_size - 32;
    if payload_end > raw_end {
        return Err(IpcError::Truncated);
    }
    Ok(Request {
        kind,
        command_id: word(input, cmif + 8)?,
        pid,
        data: input[payload_start..payload_end].to_vec(),
    })
}

/// Construct a complete cleared TLS reply, with optional outgoing move handles.
pub fn encode_reply(
    result: u32,
    data: &[u8],
    moved: &[u32],
) -> Result<[u8; IPC_BUFFER_SIZE], IpcError> {
    if data.len() % 4 != 0 {
        return Err(IpcError::UnalignedPayload);
    }
    if moved.len() > 15 || data.len() > IPC_BUFFER_SIZE - 32 {
        return Err(IpcError::TooLarge);
    }
    let raw_start = 8 + if moved.is_empty() {
        0
    } else {
        4 + moved.len() * 4
    };
    let raw_size = 32 + data.len();
    if raw_start + raw_size > IPC_BUFFER_SIZE {
        return Err(IpcError::TooLarge);
    }
    let mut reply = [0; IPC_BUFFER_SIZE];
    let put = |output: &mut [u8], at: usize, value: u32| {
        output[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    put(
        &mut reply,
        4,
        (raw_size / 4) as u32 | if moved.is_empty() { 0 } else { 1 << 31 },
    );
    if !moved.is_empty() {
        put(&mut reply, 8, (moved.len() as u32) << 5);
        for (index, handle) in moved.iter().enumerate() {
            put(&mut reply, 12 + index * 4, *handle);
        }
    }
    let cmif = (raw_start + 15) & !15;
    put(&mut reply, cmif, u32::from_le_bytes(*b"SFCO"));
    put(&mut reply, cmif + 8, result);
    reply[cmif + 16..cmif + 16 + data.len()].copy_from_slice(data);
    Ok(reply)
}
