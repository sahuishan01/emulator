use emulator_ipc::{encode_reply, parse_request, CommandType, IpcError};

fn request(pid: bool, data: &[u8]) -> Vec<u8> {
    let start = if pid { 20 } else { 8 };
    let cmif = (start + 15) & !15;
    let mut bytes = vec![0; start + 32 + data.len()];
    bytes[..4].copy_from_slice(&4_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(
        &((8 + data.len() / 4) as u32 | if pid { 1 << 31 } else { 0 }).to_le_bytes(),
    );
    if pid {
        bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[12..20].copy_from_slice(&0x8877665544332211_u64.to_le_bytes());
    }
    bytes[cmif..cmif + 4].copy_from_slice(b"SFCI");
    bytes[cmif + 8..cmif + 12].copy_from_slice(&7_u32.to_le_bytes());
    bytes[cmif + 16..cmif + 16 + data.len()].copy_from_slice(data);
    bytes
}

#[test]
fn known_wire_requests_respect_pid_alignment_and_total_padding() {
    for pid in [false, true] {
        let bytes = request(pid, &42_u64.to_le_bytes());
        let parsed = parse_request(&bytes).unwrap();
        assert_eq!(parsed.kind, CommandType::Request);
        assert_eq!(parsed.command_id, 7);
        assert_eq!(parsed.data, 42_u64.to_le_bytes());
        assert_eq!(
            parsed.pid,
            if pid { Some(0x8877665544332211) } else { None }
        );
        for size in 0..bytes.len() {
            assert!(parse_request(&bytes[..size]).is_err(), "prefix {size}");
        }
    }
}

#[test]
fn an_empty_special_header_has_distinct_raw_alignment_without_a_pid() {
    let mut bytes = request(false, &42_u64.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x8000000a_u32.to_le_bytes());
    bytes.resize(52, 0);
    let parsed = parse_request(&bytes).unwrap();
    assert_eq!(parsed.pid, None);
    assert_eq!(parsed.data, 42_u64.to_le_bytes());
    assert!(parse_request(&bytes[..51]).is_err());
}

#[test]
fn unsupported_descriptors_handles_and_context_never_parse_as_success() {
    for (offset, value) in [
        (0, 0x10004_u32),
        (0, 6),
        (4, 0x400),
        (4, 0x3ff),
        (16, 0),
        (20, 1),
        (28, 1),
    ] {
        let mut bytes = request(false, &[]);
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(parse_request(&bytes).is_err(), "offset {offset}={value:#x}");
    }
    let mut bytes = request(true, &[]);
    bytes[8..12].copy_from_slice(&3_u32.to_le_bytes()); // Incoming copy handle.
    assert_eq!(parse_request(&bytes), Err(IpcError::UnsupportedLayout));
    assert_eq!(parse_request(&[0; 257]), Err(IpcError::TooLarge));
}

#[test]
fn replies_have_golden_sfco_layout_and_move_handle_placement() {
    let reply = encode_reply(0x1234, &42_u64.to_le_bytes(), &[]).unwrap();
    assert_eq!(&reply[..8], &[0, 0, 0, 0, 10, 0, 0, 0]);
    assert_eq!(
        &reply[16..32],
        &[b'S', b'F', b'C', b'O', 0, 0, 0, 0, 0x34, 0x12, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(&reply[32..40], &42_u64.to_le_bytes());
    assert!(reply[40..].iter().all(|b| *b == 0));
    let moved = encode_reply(0, &[], &[0x12345678]).unwrap();
    assert_eq!(
        &moved[4..16],
        &[8, 0, 0, 0x80, 0x20, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]
    );
    assert_eq!(&moved[16..20], b"SFCO");
    assert!(encode_reply(0, &[0; 225], &[]).is_err());
    assert!(encode_reply(0, &[], &[1; 16]).is_err());
}
