use emulator_loader::pfs0::{Pfs0Archive, PFS0_MAGIC};

fn build_pfs0(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(PFS0_MAGIC);
    header.extend_from_slice(&(files.len() as u32).to_le_bytes());

    let mut string_table = Vec::new();
    let mut file_entries = Vec::new();
    let mut current_data_offset = 0u64;

    for (name, data) in files {
        let name_offset = string_table.len() as u32;
        string_table.extend_from_slice(name.as_bytes());
        string_table.push(0);

        file_entries.extend_from_slice(&current_data_offset.to_le_bytes());
        file_entries.extend_from_slice(&(data.len() as u64).to_le_bytes());
        file_entries.extend_from_slice(&name_offset.to_le_bytes());
        file_entries.extend_from_slice(&0u32.to_le_bytes()); // reserved

        current_data_offset += data.len() as u64;
    }

    header.extend_from_slice(&(string_table.len() as u32).to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); // reserved

    let mut archive = Vec::new();
    archive.extend_from_slice(&header);
    archive.extend_from_slice(&file_entries);
    archive.extend_from_slice(&string_table);

    for (_, data) in files {
        archive.extend_from_slice(data);
    }

    archive
}

#[test]
fn test_pfs0_parse_and_extract() {
    let files = [
        ("main.nso", b"dummy nso payload 1".as_slice()),
        ("main.npdm", b"dummy npdm payload".as_slice()),
        ("subsdk0.nso", b"dummy subsdk0 payload 2".as_slice()),
    ];

    let archive_bytes = build_pfs0(&files);
    let archive = Pfs0Archive::parse(&archive_bytes).expect("Failed to parse PFS0");

    assert_eq!(archive.files().len(), 3);
    assert_eq!(archive.files()[0].name, "main.nso");
    assert_eq!(archive.files()[1].name, "main.npdm");
    assert_eq!(archive.files()[2].name, "subsdk0.nso");

    assert_eq!(archive.get_file("main.nso"), Some(b"dummy nso payload 1".as_slice()));
    assert_eq!(archive.get_file("main.npdm"), Some(b"dummy npdm payload".as_slice()));
    assert_eq!(archive.get_file("subsdk0.nso"), Some(b"dummy subsdk0 payload 2".as_slice()));
    assert_eq!(archive.get_file("nonexistent.nso"), None);
}

#[test]
fn test_pfs0_invalid_magic() {
    let mut bytes = build_pfs0(&[("test", b"abc")]);
    bytes[0] = b'X';
    assert!(Pfs0Archive::parse(&bytes).is_err());
}
