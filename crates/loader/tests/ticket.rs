use emulator_crypto::aes128::aes128_ecb_encrypt;
use emulator_crypto::keys::KeyFile;
use emulator_loader::ticket::Ticket;

fn make_test_ticket(
    sig_type: u32,
    issuer: &str,
    rights_id: [u8; 16],
    title_key_type: u8,
    master_key_rev: u8,
    encrypted_key: [u8; 16],
) -> Vec<u8> {
    let (sig_len, pad_len) = match sig_type {
        0x010000 | 0x010003 => (0x200, 0x3C),
        0x010001 | 0x010004 => (0x100, 0x3C),
        0x010002 | 0x010005 => (0x3C, 0x40),
        0x010006 => (0x14, 0x28),
        _ => panic!("unsupported test sig type"),
    };

    let mut ticket = Vec::new();
    // 0x00..0x04: sig type
    ticket.extend_from_slice(&sig_type.to_be_bytes());
    // sig
    ticket.resize(ticket.len() + sig_len, 0xAA);
    // pad
    ticket.resize(ticket.len() + pad_len, 0);

    let body_start = ticket.len();
    ticket.resize(body_start + 0x180, 0);

    // Issuer (0x00..0x40)
    let issuer_bytes = issuer.as_bytes();
    ticket[body_start..body_start + issuer_bytes.len()].copy_from_slice(issuer_bytes);

    // Title key block (0x40..0x140)
    ticket[body_start + 0x40..body_start + 0x50].copy_from_slice(&encrypted_key);

    // Format version (0x140)
    ticket[body_start + 0x140] = 2;

    // Title key type (0x141)
    ticket[body_start + 0x141] = title_key_type;

    // Master key revision (0x145)
    ticket[body_start + 0x145] = master_key_rev;

    // Ticket ID (0x150..0x158)
    ticket[body_start + 0x150..body_start + 0x158].copy_from_slice(&123456789u64.to_le_bytes());

    // Rights ID (0x160..0x170)
    ticket[body_start + 0x160..body_start + 0x170].copy_from_slice(&rights_id);

    ticket
}

#[test]
fn test_ticket_parsing_and_signature_types() {
    let rights_id = [0x55; 16];
    let key = [0x77; 16];

    for sig_type in [0x010000, 0x010001, 0x010002, 0x010003, 0x010004, 0x010005, 0x010006] {
        let raw = make_test_ticket(sig_type, "Root-CA00000003-XS0000000c", rights_id, 0, 1, key);
        let parsed = Ticket::parse(&raw).expect("ticket should parse successfully");
        assert_eq!(parsed.signature_type, sig_type);
        assert_eq!(parsed.issuer, "Root-CA00000003-XS0000000c");
        assert_eq!(parsed.rights_id, rights_id);
        assert_eq!(parsed.title_key_type, 0);
        assert_eq!(parsed.master_key_revision, 1);
        assert_eq!(parsed.ticket_id, 123456789);
        assert!(parsed.is_common());
    }
}

#[test]
fn test_ticket_common_title_key_decryption_and_import() {
    let rights_id = [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01];
    let raw_title_key = [0x42; 16];
    let titlekek = [0x88; 16];

    // Encrypt title_key with titlekek via AES-128-ECB
    let mut encrypted_title_key = raw_title_key;
    aes128_ecb_encrypt(&titlekek, &mut encrypted_title_key).unwrap();

    let raw_ticket = make_test_ticket(
        0x010004, // RSA-2048 SHA-256
        "Root-CA",
        rights_id,
        0, // Common
        2, // Master key revision 2 -> titlekek_02
        encrypted_title_key,
    );

    let ticket = Ticket::parse(&raw_ticket).expect("ticket should parse");

    // Keys without titlekek_02 should fail decryption
    let mut keys = KeyFile::default();
    assert!(ticket.decrypt_title_key(&keys).is_err());

    // Register titlekek_02
    keys.add_key("titlekek_02", &titlekek).unwrap();

    // Now decryption should succeed and match raw_title_key
    let decrypted = ticket.decrypt_title_key(&keys).expect("decryption should succeed");
    assert_eq!(decrypted, raw_title_key);

    // Import into KeyFile
    ticket.import_into(&mut keys).expect("import should succeed");

    // Look up title key in KeyFile by rights ID
    let looked_up = keys.title_key(&rights_id).expect("title key must be registered");
    assert_eq!(looked_up, &raw_title_key);
}

#[test]
fn test_ticket_rejects_personalized_and_truncated() {
    let rights_id = [0x11; 16];
    let key = [0x22; 16];

    // Personalized ticket (title_key_type == 1)
    let raw = make_test_ticket(0x010004, "Root", rights_id, 1, 0, key);
    let ticket = Ticket::parse(&raw).unwrap();
    assert!(!ticket.is_common());
    let keys = KeyFile::default();
    assert!(ticket.decrypt_title_key(&keys).is_err());

    // Truncated tickets
    assert!(Ticket::parse(&[]).is_err());
    assert!(Ticket::parse(&raw[..10]).is_err());
    assert!(Ticket::parse(&raw[..raw.len() - 20]).is_err());
}
