use emulator_loader::cnmt::{Cnmt, CnmtContentType, ContentMetaType};

fn make_test_cnmt(
    title_id: u64,
    version: u32,
    meta_type: u8,
    records: &[(CnmtContentType, [u8; 16], u64)],
) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&title_id.to_le_bytes());
    data.extend_from_slice(&version.to_le_bytes());
    data.push(meta_type); // ContentMetaType
    data.push(0); // platform / reserved
    data.extend_from_slice(&0u16.to_le_bytes()); // ext_header_size
    data.extend_from_slice(&(records.len() as u16).to_le_bytes()); // content_count
    data.extend_from_slice(&0u16.to_le_bytes()); // content_meta_count
    data.push(0); // attributes
    data.extend_from_slice(&[0, 0, 0]); // reserved
    data.extend_from_slice(&65536u32.to_le_bytes()); // required system version
    data.extend_from_slice(&[0, 0, 0, 0]); // reserved

    assert_eq!(data.len(), 0x20);

    for (content_type, content_id, size) in records {
        // 0x00..0x20: SHA-256 hash
        data.resize(data.len() + 32, 0xEE);
        // 0x20..0x30: Content ID
        data.extend_from_slice(content_id);
        // 0x30..0x36: Size (6 bytes)
        let size_bytes = size.to_le_bytes();
        data.extend_from_slice(&size_bytes[..6]);
        // 0x36: ContentType
        data.push(*content_type as u8);
        // 0x37: IdOffset
        data.push(0);
    }

    data
}

#[test]
fn test_cnmt_parsing_and_record_lookup() {
    let prog_id = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10];
    let ctrl_id = [0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30];
    let legal_id = [0x31; 16];

    let raw = make_test_cnmt(
        0x0100123456789000,
        0x10000,
        ContentMetaType::Application as u8,
        &[
            (CnmtContentType::Program, prog_id, 1048576),
            (CnmtContentType::Control, ctrl_id, 65536),
            (CnmtContentType::LegalInformation, legal_id, 32768),
        ],
    );

    let cnmt = Cnmt::parse(&raw).expect("cnmt should parse successfully");
    assert_eq!(cnmt.title_id, 0x0100123456789000);
    assert_eq!(cnmt.version, 0x10000);
    assert_eq!(cnmt.meta_type, ContentMetaType::Application);
    assert_eq!(cnmt.contents.len(), 3);

    let prog = cnmt.find_program_record().expect("must find program record");
    assert_eq!(prog.content_id, prog_id);
    assert_eq!(prog.size, 1048576);
    assert_eq!(prog.content_id_hex(), "0102030405060708090a0b0c0d0e0f10");
    assert_eq!(prog.nca_filename(), "0102030405060708090a0b0c0d0e0f10.nca");

    let ctrl = cnmt.find_control_record().expect("must find control record");
    assert_eq!(ctrl.content_id, ctrl_id);
    assert_eq!(ctrl.nca_filename(), "2122232425262728292a2b2c2d2e2f30.nca");

    let legal = cnmt.find_legal_info_record().expect("must find legal info record");
    assert_eq!(legal.content_id, legal_id);
}

#[test]
fn test_cnmt_rejects_truncated() {
    assert!(Cnmt::parse(&[]).is_err());
    assert!(Cnmt::parse(&[0; 0x1F]).is_err());
    let incomplete = make_test_cnmt(1, 1, 1, &[(CnmtContentType::Program, [0; 16], 100)]);
    assert!(Cnmt::parse(&incomplete[..incomplete.len() - 10]).is_err());
}
