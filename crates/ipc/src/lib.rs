//! Original bounded HIPC/CMIF v0 with normal A/B map-alias buffer descriptors,
//! plus the headerless TIPC framing (message type = 16 + command) that newer
//! guests use for `sm:`, X pointer (static send) descriptors and C receive-list
//! descriptors. Exchange buffers, inline receive lists, domains and incoming
//! move handles remain unsupported.

use std::fmt;

pub const IPC_BUFFER_SIZE: usize = 0x100;
pub const MAX_ALIAS_BUFFERS: usize = 4;
pub const MAX_COPY_HANDLES: usize = 4;
pub const MAX_POINTER_BUFFERS: usize = 4;
/// TIPC message types are `TIPC_COMMAND_BASE + command id`.
pub const TIPC_COMMAND_BASE: u16 = 16;
const TIPC_COMMAND_LAST: u16 = TIPC_COMMAND_BASE + 0xff;

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
    BufferLimit,
    HandleLimit,
    UnsupportedBufferMode(u8),
    UnsupportedDomainCommand(u8),
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
    /// The client is ending the session; there is no payload and no reply.
    Close,
}

/// Normal-mode HIPC map-alias descriptor: 58-bit address and 36-bit length.
/// Wire decoding grants no memory rights; the kernel validates the whole range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferDescriptor {
    pub address: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: CommandType,
    /// TIPC framing: the reply must be encoded with [`encode_tipc_reply`].
    pub tipc: bool,
    pub command_id: u32,
    /// The kernel replaces this untrusted wire value with the sender's identity.
    pub pid: Option<u64>,
    pub data: Vec<u8>,
    pub send_buffers: Vec<BufferDescriptor>,
    pub receive_buffers: Vec<BufferDescriptor>,
    /// X descriptors: guest data sent through the pointer buffer.
    pub pointer_send: Vec<BufferDescriptor>,
    /// C descriptors: guest receive list for pointer-buffer output.
    pub pointer_receive: Vec<BufferDescriptor>,
    pub copy_handles: Vec<u32>,
    /// Present when the message was framed for a CMIF domain session.
    pub domain: Option<DomainRequest>,
}

/// CMIF domain framing: which object the message addresses, and whether it
/// is a call or a request to drop the object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainRequest {
    pub object_id: u32,
    pub close: bool,
    pub input_objects: Vec<u32>,
}
pub const MAX_DOMAIN_INPUT_OBJECTS: usize = 8;

fn word(input: &[u8], offset: usize) -> Result<u32, IpcError> {
    let bytes = input.get(offset..offset + 4).ok_or(IpcError::Truncated)?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
}

pub fn parse_request(input: &[u8]) -> Result<Request, IpcError> {
    parse_session_request(input, false)
}

/// Parse a message sent to a session. On a domain session every non-control,
/// non-TIPC request carries a 16-byte domain header ahead of the CMIF header.
pub fn parse_session_request(input: &[u8], domain: bool) -> Result<Request, IpcError> {
    if input.len() > IPC_BUFFER_SIZE {
        return Err(IpcError::TooLarge);
    }
    let first = word(input, 0)?;
    let second = word(input, 4)?;
    if first & 0xf000_0000 != 0 || second & 0x7fff_c000 != 0 {
        return Err(IpcError::UnsupportedLayout);
    }
    if first as u16 == 2 {
        if first >> 16 != 0 || second != 0 {
            return Err(IpcError::UnsupportedLayout);
        }
        return Ok(Request {
            kind: CommandType::Close,
            tipc: false,
            command_id: 0,
            pid: None,
            data: Vec::new(),
            send_buffers: Vec::new(),
            receive_buffers: Vec::new(),
            pointer_send: Vec::new(),
            pointer_receive: Vec::new(),
            copy_handles: Vec::new(),
            domain: None,
        });
    }
    // 6 and 7 are the same requests carrying a diagnostic context token.
    let with_context = matches!(first as u16, 6 | 7);
    let (kind, tipc_command) = match first as u16 {
        4 | 6 => (CommandType::Request, None),
        5 | 7 => (CommandType::Control, None),
        // 15 is the TIPC close request, which stays unsupported.
        tipc @ TIPC_COMMAND_BASE..=TIPC_COMMAND_LAST => {
            (CommandType::Request, Some(u32::from(tipc - TIPC_COMMAND_BASE)))
        }
        other => return Err(IpcError::UnsupportedType(other)),
    };
    let pointer_count = ((first >> 16) & 15) as usize;
    // 0: none, 1: inline in the raw area (unsupported), 2: one entry,
    // n > 2: n - 2 entries.
    let receive_list = match (second >> 10) & 15 {
        0 => 0,
        1 => return Err(IpcError::UnsupportedLayout),
        2 => 1,
        count => (count - 2) as usize,
    };
    if pointer_count > MAX_POINTER_BUFFERS || receive_list > MAX_POINTER_BUFFERS {
        return Err(IpcError::BufferLimit);
    }
    let send_count = ((first >> 20) & 15) as usize;
    let receive_count = ((first >> 24) & 15) as usize;
    if send_count + receive_count > MAX_ALIAS_BUFFERS {
        return Err(IpcError::BufferLimit);
    }
    if kind == CommandType::Control
        && send_count + receive_count + pointer_count + receive_list != 0
    {
        return Err(IpcError::UnsupportedLayout);
    }
    let mut raw_start = 8;
    let mut copy_handles = Vec::new();
    let pid = if second >> 31 != 0 {
        let special = word(input, raw_start)?;
        raw_start += 4;
        if special & !0x1f != 0 {
            return Err(IpcError::UnsupportedLayout);
        }
        let copies = ((special >> 1) & 15) as usize;
        if copies > MAX_COPY_HANDLES {
            return Err(IpcError::HandleLimit);
        }
        if kind == CommandType::Control && copies != 0 {
            return Err(IpcError::UnsupportedLayout);
        }
        let pid = if special & 1 == 0 {
            None
        } else {
            let bytes = input
                .get(raw_start..raw_start + 8)
                .ok_or(IpcError::Truncated)?;
            raw_start += 8;
            Some(u64::from_le_bytes(bytes.try_into().expect("eight bytes")))
        };
        for _ in 0..copies {
            copy_handles.push(word(input, raw_start)?);
            raw_start += 4;
        }
        pid
    } else {
        None
    };
    let mut pointer_send = Vec::new();
    for _ in 0..pointer_count {
        let fields = word(input, raw_start)?;
        let low_address = word(input, raw_start + 4)?;
        pointer_send.push(BufferDescriptor {
            address: u64::from(low_address)
                | (u64::from((fields >> 12) & 15) << 32)
                | (u64::from((fields >> 6) & 0x3f) << 36),
            size: u64::from(fields >> 16),
        });
        raw_start += 8;
    }
    let mut descriptors = |count: usize| -> Result<Vec<BufferDescriptor>, IpcError> {
        let mut buffers = Vec::new();
        for _ in 0..count {
            let low_size = word(input, raw_start)?;
            let low_address = word(input, raw_start + 4)?;
            let fields = word(input, raw_start + 8)?;
            // Mode 0 Normal, 1 NonSecure, 3 NonDevice only select the server's
            // mapping attribute; the data path is identical. Mode 2 is invalid.
            if fields & 3 == 2 {
                return Err(IpcError::UnsupportedBufferMode((fields & 3) as u8));
            }
            buffers.push(BufferDescriptor {
                address: u64::from(low_address)
                    | (u64::from(fields >> 28) << 32)
                    | (u64::from((fields >> 2) & 0x3f_ffff) << 36),
                size: u64::from(low_size) | (u64::from((fields >> 24) & 15) << 32),
            });
            raw_start += 12;
        }
        Ok(buffers)
    };
    let send_buffers = descriptors(send_count)?;
    let receive_buffers = descriptors(receive_count)?;
    let raw_size = ((second & 0x3ff) as usize) * 4;
    let mut pointer_receive = Vec::new();
    for index in 0..receive_list {
        let at = raw_start + raw_size + index * 8;
        let fields = word(input, at + 4)?;
        pointer_receive.push(BufferDescriptor {
            address: u64::from(word(input, at)?) | (u64::from(fields & 0xffff) << 32),
            size: u64::from(fields >> 16),
        });
    }
    if let Some(command_id) = tipc_command {
        // TIPC carries no CMIF header or alignment padding: the raw words are
        // the payload and the command lives in the message type.
        let raw_end = raw_start + raw_size;
        if raw_end > IPC_BUFFER_SIZE {
            return Err(IpcError::TooLarge);
        }
        if raw_end > input.len() {
            return Err(IpcError::Truncated);
        }
        return Ok(Request {
            kind,
            tipc: true,
            command_id,
            pid,
            data: input[raw_start..raw_end].to_vec(),
            send_buffers,
            receive_buffers,
            pointer_send,
            pointer_receive,
            copy_handles,
            domain: None,
        });
    }
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
    let mut cmif = (raw_start + 15) & !15;
    let mut payload_size = raw_size - 32;
    let mut domain_request = None;
    if domain && kind == CommandType::Request {
        let header = word(input, cmif)?;
        let object_id = word(input, cmif + 4)?;
        let data_size = (header >> 16) as usize;
        let object_count = ((header >> 8) & 0xff) as usize;
        if object_count > MAX_DOMAIN_INPUT_OBJECTS {
            return Err(IpcError::HandleLimit);
        }
        // Domain header, CMIF bytes, then the input object IDs.
        let objects_at = cmif + 16 + data_size;
        if objects_at + object_count * 4 > raw_end {
            return Err(IpcError::Truncated);
        }
        let input_objects = (0..object_count)
            .map(|index| word(input, objects_at + index * 4))
            .collect::<Result<Vec<_>, _>>()?;
        let close = match header & 0xff {
            1 => false,
            2 => true,
            other => return Err(IpcError::UnsupportedDomainCommand(other as u8)),
        };
        if close {
            return Ok(Request {
                kind,
                tipc: false,
                command_id: 0,
                pid,
                data: Vec::new(),
                send_buffers,
                receive_buffers,
                pointer_send,
                pointer_receive,
                copy_handles,
                domain: Some(DomainRequest {
                    object_id,
                    close,
                    input_objects,
                }),
            });
        }
        if data_size < 16 {
            return Err(IpcError::Truncated);
        }
        cmif += 16;
        payload_size = data_size - 16;
        domain_request = Some(DomainRequest {
            object_id,
            close,
            input_objects,
        });
    }
    if word(input, cmif)? != u32::from_le_bytes(*b"SFCI") {
        return Err(IpcError::InvalidMagic);
    }
    let version = word(input, cmif + 4)?;
    if version != u32::from(with_context) {
        return Err(IpcError::UnsupportedVersion(version));
    }
    if word(input, cmif + 12)? != 0 && !with_context {
        return Err(IpcError::UnsupportedContext);
    }
    // CMIF reserves 16 bytes total alignment padding, independent of where
    // its raw data starts, plus its own 16-byte input header.
    let payload_start = cmif + 16;
    let payload_end = payload_start + payload_size;
    if payload_end > raw_end {
        return Err(IpcError::Truncated);
    }
    Ok(Request {
        kind,
        tipc: false,
        command_id: word(input, cmif + 8)?,
        pid,
        data: input[payload_start..payload_end].to_vec(),
        send_buffers,
        receive_buffers,
        pointer_send,
        pointer_receive,
        copy_handles,
        domain: domain_request,
    })
}

/// Construct a complete cleared TLS reply, with optional outgoing move handles.
pub fn encode_reply(
    result: u32,
    data: &[u8],
    moved: &[u32],
) -> Result<[u8; IPC_BUFFER_SIZE], IpcError> {
    encode_session_reply(result, data, &[], moved, None)
}

/// As [`encode_reply`], with copy handles ahead of the move handles.
/// `domain_objects` selects domain framing: a 16-byte output header counting
/// the new object IDs, which follow the payload.
pub fn encode_session_reply(
    result: u32,
    data: &[u8],
    copied: &[u32],
    moved: &[u32],
    domain_objects: Option<&[u32]>,
) -> Result<[u8; IPC_BUFFER_SIZE], IpcError> {
    if data.len() % 4 != 0 {
        return Err(IpcError::UnalignedPayload);
    }
    if moved.len() > 15 || copied.len() > 15 || data.len() > IPC_BUFFER_SIZE - 32 {
        return Err(IpcError::TooLarge);
    }
    let handles = copied.len() + moved.len();
    let raw_start = 8 + if handles == 0 { 0 } else { 4 + handles * 4 };
    let domain_size = domain_objects.map_or(0, |objects| 16 + objects.len() * 4);
    let raw_size = 32 + data.len() + domain_size;
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
        (raw_size / 4) as u32 | if handles == 0 { 0 } else { 1 << 31 },
    );
    if handles != 0 {
        put(
            &mut reply,
            8,
            (copied.len() as u32) << 1 | (moved.len() as u32) << 5,
        );
        for (index, handle) in copied.iter().chain(moved).enumerate() {
            put(&mut reply, 12 + index * 4, *handle);
        }
    }
    let mut cmif = (raw_start + 15) & !15;
    if let Some(objects) = domain_objects {
        put(&mut reply, cmif, objects.len() as u32);
        cmif += 16;
        for (index, object) in objects.iter().enumerate() {
            put(&mut reply, cmif + 16 + data.len() + index * 4, *object);
        }
    }
    put(&mut reply, cmif, u32::from_le_bytes(*b"SFCO"));
    put(&mut reply, cmif + 8, result);
    reply[cmif + 16..cmif + 16 + data.len()].copy_from_slice(data);
    Ok(reply)
}

/// Construct a complete cleared TLS reply in TIPC framing: the raw words are
/// the result code followed by the payload, with no CMIF header or padding.
pub fn encode_tipc_reply(
    result: u32,
    data: &[u8],
    moved: &[u32],
) -> Result<[u8; IPC_BUFFER_SIZE], IpcError> {
    if data.len() % 4 != 0 {
        return Err(IpcError::UnalignedPayload);
    }
    if moved.len() > 15 {
        return Err(IpcError::TooLarge);
    }
    let raw_start = 8 + if moved.is_empty() {
        0
    } else {
        4 + moved.len() * 4
    };
    let raw_size = 4 + data.len();
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
    put(&mut reply, raw_start, result);
    reply[raw_start + 4..raw_start + 4 + data.len()].copy_from_slice(data);
    Ok(reply)
}
