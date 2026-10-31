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
    bytes[8..12].copy_from_slice(&0x21_u32.to_le_bytes()); // Incoming move handle.
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

#[test]
fn normal_alias_descriptors_preserve_all_address_and_length_bits() {
    let words = [
        0x0110_0004_u32,
        10,
        0x89ab_cdef,
        0x7654_3210,
        0xba48_d158,
        0x1122_3344,
        0x5566_7788,
        0x9c00_0000,
        0x4943_4653,
        0,
        7,
        0,
        0x1234_5678,
        0,
        0,
        0,
    ];
    let mut bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    bytes.resize(72, 0); // Full 16-byte CMIF padding, including aligned raw data.
    let parsed = parse_request(&bytes).unwrap();
    assert_eq!(
        parsed.send_buffers,
        [emulator_ipc::BufferDescriptor {
            address: 0x0123_456b_7654_3210,
            size: 0x0a_89ab_cdef
        }]
    );
    assert_eq!(
        parsed.receive_buffers,
        [emulator_ipc::BufferDescriptor {
            address: 0x9_5566_7788,
            size: 0x0c_1122_3344
        }]
    );
    assert_eq!(parsed.data, [0x78, 0x56, 0x34, 0x12, 0, 0, 0, 0]);
}
#[test]
fn alias_descriptors_follow_special_headers_and_require_complete_raw_data() {
    for special in [0_u32, 1, 2] {
        let mut bytes = vec![0; 256];
        bytes[..4].copy_from_slice(&0x0130_0004_u32.to_le_bytes());
        bytes[4..8]
            .copy_from_slice(&(14 | if special != 0 { 1_u32 << 31 } else { 0 }).to_le_bytes());
        let mut at = 8;
        if special != 0 {
            bytes[8..12].copy_from_slice(&(special - 1).to_le_bytes());
            at = 12;
        }
        if special == 2 {
            bytes[12..20].copy_from_slice(&55_u64.to_le_bytes());
            at = 20;
        }
        for address in [0x1000_u32, 0x2000, 0x3000, 0x4000] {
            bytes[at..at + 4].copy_from_slice(&8_u32.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&address.to_le_bytes());
            at += 12;
        }
        let raw_end = at + 56;
        let cmif = (at + 15) & !15;
        bytes[cmif..cmif + 4].copy_from_slice(b"SFCI");
        bytes[cmif + 8..cmif + 12].copy_from_slice(&2_u32.to_le_bytes());
        bytes[cmif + 16..cmif + 40].fill(0x55);
        bytes.truncate(raw_end);
        let parsed = parse_request(&bytes).unwrap();
        assert_eq!(
            (parsed.send_buffers.len(), parsed.receive_buffers.len()),
            (3, 1)
        );
        assert_eq!(parsed.data, vec![0x55; 24]);
        assert_eq!(parsed.pid, if special == 2 { Some(55) } else { None });
        for length in 0..bytes.len() {
            assert!(
                parse_request(&bytes[..length]).is_err(),
                "special {special}, prefix {length}"
            );
        }
    }
}
#[test]
fn unsupported_alias_modes_excess_counts_and_control_buffers_are_rejected() {
    let mut bytes = vec![0; 64];
    bytes[..4].copy_from_slice(&0x0110_0004_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&8_u32.to_le_bytes());
    bytes[32..36].copy_from_slice(b"SFCI");
    for mode in [1_u32, 2, 3] {
        bytes[16..20].copy_from_slice(&mode.to_le_bytes());
        assert_eq!(
            parse_request(&bytes),
            Err(IpcError::UnsupportedBufferMode(mode as u8))
        );
    }
    bytes[16..20].fill(0);
    bytes[..4].copy_from_slice(&0x0140_0004_u32.to_le_bytes());
    assert_eq!(parse_request(&bytes), Err(IpcError::BufferLimit));
    bytes[..4].copy_from_slice(&0x0110_0005_u32.to_le_bytes());
    assert_eq!(parse_request(&bytes), Err(IpcError::UnsupportedLayout));
    for first in [0x0001_0004_u32, 0x1000_0004] {
        bytes[..4].copy_from_slice(&first.to_le_bytes());
        assert_eq!(parse_request(&bytes), Err(IpcError::UnsupportedLayout));
    }
}
#[test]
fn copied_handles_precede_descriptors_and_preserve_pid_and_cmif_alignment() {
    for pid in [false, true] {
        let mut bytes = vec![0; 256];
        bytes[..4].copy_from_slice(&0x0110_0004_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&0x80000009_u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(4_u32 | u32::from(pid)).to_le_bytes());
        let mut at = 12;
        if pid {
            bytes[at..at + 8].copy_from_slice(&99_u64.to_le_bytes());
            at += 8;
        }
        for handle in [0xffff8001_u32, 0x12345678] {
            bytes[at..at + 4].copy_from_slice(&handle.to_le_bytes());
            at += 4;
        }
        for address in [0x4000_u32, 0x5000] {
            bytes[at..at + 4].copy_from_slice(&8_u32.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&address.to_le_bytes());
            at += 12;
        }
        let end = at + 36;
        let cmif = (at + 15) & !15;
        bytes[cmif..cmif + 4].copy_from_slice(b"SFCI");
        bytes[cmif + 8..cmif + 12].copy_from_slice(&3_u32.to_le_bytes());
        bytes[cmif + 16..cmif + 20].copy_from_slice(&4096_u32.to_le_bytes());
        bytes.truncate(end);
        let parsed = parse_request(&bytes).unwrap();
        assert_eq!(parsed.copy_handles, [0xffff8001, 0x12345678]);
        assert_eq!(parsed.pid, if pid { Some(99) } else { None });
        assert_eq!(parsed.data, 4096_u32.to_le_bytes());
        assert_eq!(parsed.send_buffers[0].address, 0x4000);
        assert_eq!(parsed.receive_buffers[0].address, 0x5000);
        for length in 0..bytes.len() {
            assert!(parse_request(&bytes[..length]).is_err());
        }
    }
}
#[test]
fn copied_handle_limits_moves_and_control_transfers_remain_explicit_errors() {
    let mut bytes = vec![0; 64];
    bytes[..4].copy_from_slice(&4_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x80000008_u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&10_u32.to_le_bytes());
    assert_eq!(parse_request(&bytes), Err(IpcError::HandleLimit));
    for special in [0x20_u32, 0x40, 0x10000] {
        bytes[8..12].copy_from_slice(&special.to_le_bytes());
        assert_eq!(parse_request(&bytes), Err(IpcError::UnsupportedLayout));
    }
    bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
    bytes[..4].copy_from_slice(&5_u32.to_le_bytes());
    assert_eq!(parse_request(&bytes), Err(IpcError::UnsupportedLayout));
}
