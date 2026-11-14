use emulator_crypto::keys::KeyFile;
use emulator_loader::nsp::NspArchive;
use emulator_loader::LoadError;

fn build_pfs0(files: &[(&str, &[u8])]) -> Vec<u8> {
    let num_files = files.len() as u32;
    let mut string_table = Vec::new();
    let mut file_entries = Vec::new();
    let mut current_offset = 0u64;

    for (name, content) in files {
        let name_offset = string_table.len() as u32;
        string_table.extend_from_slice(name.as_bytes());
        string_table.push(0);

        let file_offset = current_offset;
        let file_size = content.len() as u64;
        current_offset += file_size;

        file_entries.extend_from_slice(&file_offset.to_le_bytes());
        file_entries.extend_from_slice(&file_size.to_le_bytes());
        file_entries.extend_from_slice(&name_offset.to_le_bytes());
        file_entries.extend_from_slice(&0u32.to_le_bytes());
    }

    let header_size = 0x10 + file_entries.len() + string_table.len();
    let padding = (512 - (header_size % 512)) % 512;
    let string_table_size = (string_table.len() + padding) as u32;
    string_table.resize(string_table.len() + padding, 0);

    let mut pfs0 = Vec::new();
    pfs0.extend_from_slice(b"PFS0");
    pfs0.extend_from_slice(&num_files.to_le_bytes());
    pfs0.extend_from_slice(&string_table_size.to_le_bytes());
    pfs0.extend_from_slice(&0u32.to_le_bytes());
    pfs0.extend_from_slice(&file_entries);
    pfs0.extend_from_slice(&string_table);

    for (_, content) in files {
        pfs0.extend_from_slice(content);
    }
    pfs0
}

#[test]
fn test_nsp_rejects_empty_archive() {
    let data = build_pfs0(&[]);
    let nsp = NspArchive::parse(&data).expect("PFS0 parses");
    let mut keys = KeyFile::default();
    keys.add_key("header_key", &[0x11; 32]).unwrap();

    let err = nsp.find_program_nca(&keys).unwrap_err();
    assert!(
        matches!(err, LoadError::Invalid(msg) if msg.contains("empty")),
        "Expected empty archive error, got {:?}",
        err
    );
}

#[test]
fn test_nsp_detects_ncz_compressed_format() {
    let data = build_pfs0(&[
        ("0100000000001000.ncz", b"dummy ncz content"),
        ("0100000000001000.cnmt.nca", b"dummy cnmt"),
    ]);
    let nsp = NspArchive::parse(&data).expect("PFS0 parses");
    let mut keys = KeyFile::default();
    keys.add_key("header_key", &[0x11; 32]).unwrap();

    let err = nsp.find_program_nca(&keys).unwrap_err();
    assert!(
        matches!(err, LoadError::Invalid(msg) if msg.contains(".ncz / NSZ format")),
        "Expected NSZ detection error, got {:?}",
        err
    );
}

#[test]
fn test_nsp_rejects_no_nca_files() {
    let data = build_pfs0(&[
        ("readme.txt", b"instructions"),
        ("installer.exe", b"fake installer"),
    ]);
    let nsp = NspArchive::parse(&data).expect("PFS0 parses");
    let mut keys = KeyFile::default();
    keys.add_key("header_key", &[0x11; 32]).unwrap();

    let err = nsp.find_program_nca(&keys).unwrap_err();
    assert!(
        matches!(err, LoadError::Invalid(msg) if msg.contains("contains no .nca content files")),
        "Expected no NCA files error, got {:?}",
        err
    );
}

#[test]
fn test_nsp_detects_corrupt_nca_header_decryption_failure() {
    // 0xC00 bytes of dummy data where decrypted bytes do not start with NCA3 magic
    let dummy_nca = vec![0xAAu8; 0xC00];
    let data = build_pfs0(&[("0100000000001000.nca", &dummy_nca)]);
    let nsp = NspArchive::parse(&data).expect("PFS0 parses");
    let mut keys = KeyFile::default();
    keys.add_key("header_key", &[0x11; 32]).unwrap();

    let err = nsp.find_program_nca(&keys).unwrap_err();
    assert!(
        matches!(err, LoadError::Invalid(msg) if msg.contains("NCA3 magic missing")),
        "Expected NCA3 magic missing error, got {:?}",
        err
    );
}
