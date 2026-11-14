use emulator_crypto::keys::{KeyArea, KeyFile, KeyFileError, MAX_KEY_FILE_SIZE};

#[test]
fn parses_key_families_and_resolves_typed_lookups() {
    let rights_id = "01000000000000000000000000000001";
    let contents = format!(
        "# owned fixture\nheader_key = {}\nkey_area_key_application_00 = {}\ntitlekek_02 = {}\ntitlekey_{rights_id} = {}\n",
        "11".repeat(32),
        "22".repeat(16),
        "33".repeat(16),
        "44".repeat(16),
    );
    let keys = KeyFile::parse(contents.as_bytes()).unwrap();
    assert_eq!(keys.header_key(), Some(&[0x11; 32]));
    assert_eq!(
        keys.key_area_key(KeyArea::Application, 0),
        Some(&[0x22; 16])
    );
    assert_eq!(keys.title_kek(2), Some(&[0x33; 16]));
    assert_eq!(
        keys.title_key(&[0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        Some(&[0x44; 16])
    );
}

#[test]
fn normalizes_names_and_accepts_bare_rights_id_title_key() {
    let id = "01000000000000000000000000000001";
    let keys = KeyFile::parse(
        format!(
            "HEADER_KEY = {}\n{id} = {}",
            "ab".repeat(32),
            "cd".repeat(16)
        )
        .as_bytes(),
    )
    .unwrap();
    assert!(keys.header_key().is_some());
    assert_eq!(
        keys.title_key(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        Some(&[0xcd; 16])
    );
}

#[test]
fn malformed_or_ambiguous_entries_fail_without_echoing_secret_values() {
    for (contents, expected) in [
        ("no separator", KeyFileError::InvalidLine(1)),
        (
            "bad name! = 00112233445566778899aabbccddeeff",
            KeyFileError::InvalidName(1),
        ),
        ("key = abc", KeyFileError::InvalidHex(1)),
        (
            "key = 0011223344556677",
            KeyFileError::UnsupportedKeyLength(1),
        ),
        (
            "key = 00112233445566778899aabbccddeeff\nKEY = 00112233445566778899aabbccddeeff",
            KeyFileError::DuplicateName(2),
        ),
    ] {
        assert_eq!(KeyFile::parse(contents.as_bytes()).unwrap_err(), expected);
    }
}

#[test]
fn enforces_file_and_entry_bounds_and_rejects_invalid_utf8() {
    assert_eq!(
        KeyFile::parse(&vec![b' '; MAX_KEY_FILE_SIZE + 1]).unwrap_err(),
        KeyFileError::TooLarge
    );
    assert_eq!(
        KeyFile::parse(&[0xff]).unwrap_err(),
        KeyFileError::InvalidUtf8
    );
    let too_many = (0..257)
        .map(|index| format!("key_{index} = {}\n", "00".repeat(16)))
        .collect::<String>();
    assert_eq!(
        KeyFile::parse(too_many.as_bytes()).unwrap_err(),
        KeyFileError::TooManyEntries
    );
}

#[test]
fn debug_format_never_discloses_key_material() {
    let secret = "de".repeat(16);
    let keys = KeyFile::parse(format!("fixture = {secret}").as_bytes()).unwrap();
    let debug = format!("{keys:?}");
    assert!(!debug.contains(&secret));
    assert!(debug.contains("redacted"));
}

#[test]
fn parse_lenient_handles_comments_rsa_keys_and_large_sets() {
    let mut contents = String::new();
    contents.push_str("# Full Switch dump\n");
    contents.push_str("header_key = 1111111111111111111111111111111111111111111111111111111111111111 # inline comment\n");
    contents.push_str("eticket_rsa_kek = aabbccdd # invalid 4-byte key ignored\n");
    contents.push_str("master_key_00 = 22222222222222222222222222222222\n");
    // Add 300 keys (exceeding standard 256 limit)
    for i in 1..=300 {
        contents.push_str(&format!("key_{i:04} = {}\n", "33".repeat(16)));
    }
    let keys = KeyFile::parse_lenient(contents.as_bytes());
    assert_eq!(keys.header_key(), Some(&[0x11; 32]));
    assert_eq!(keys.master_key(0), Some(&[0x22; 16]));
    assert_eq!(keys.entry_count(), 302);
}

#[test]
fn test_titlekek_and_key_area_derivation_in_parse_lenient() {
    let mut contents = String::new();
    contents.push_str("master_key_00 = 11111111111111111111111111111111\n");
    contents.push_str("titlekek_source = 22222222222222222222222222222222\n");
    contents.push_str("aes_kek_generation_source = 33333333333333333333333333333333\n");
    contents.push_str("aes_key_generation_source = 44444444444444444444444444444444\n");
    contents.push_str("key_area_key_application_source = 55555555555555555555555555555555\n");
    let keys = KeyFile::parse_lenient(contents.as_bytes());

    assert!(keys.title_kek(0).is_some());
    assert!(keys.key_area_key(emulator_crypto::keys::KeyArea::Application, 0).is_some());
}

#[test]
fn test_title_key_masked_and_prefix_lookups() {
    let mut contents = String::new();
    // Register title key with lower 8 bytes masked
    contents.push_str("titlekey_01001234567800000000000000000000 = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n");
    // Register another title key with 16-hex title ID prefix
    contents.push_str("0100abcdef120000 = bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n");
    let keys = KeyFile::parse_lenient(contents.as_bytes());

    // Exact query with unmasked rights ID ending in 0000000000000001
    let rights_id_1: [u8; 16] = [
        0x01, 0x00, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    ];
    assert_eq!(keys.title_key(&rights_id_1), Some(&[0xAA; 16]));

    // Query for 16-hex prefix
    let rights_id_2: [u8; 16] = [
        0x01, 0x00, 0xab, 0xcd, 0xef, 0x12, 0x00, 0x00,
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
    ];
    assert_eq!(keys.title_key(&rights_id_2), Some(&[0xBB; 16]));
}
