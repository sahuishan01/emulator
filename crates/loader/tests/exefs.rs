use emulator_loader::exefs::ExeFsProgram;
use emulator_loader::npdm::{NPDM_HEADER_SIZE, NPDM_MAGIC};
use emulator_loader::pfs0::PFS0_MAGIC;

fn build_test_nso() -> Vec<u8> {
    let mut data = vec![0u8; 0x100 + 0x3000];
    data[0..4].copy_from_slice(b"NSO0");
    data[4..8].copy_from_slice(&0u32.to_le_bytes()); // version
    data[8..12].copy_from_slice(&0u32.to_le_bytes()); // flags: uncompressed

    // Text: file offset 0x100, memory offset 0x0, decompressed size 0x1000
    data[0x10..0x14].copy_from_slice(&0x100u32.to_le_bytes());
    data[0x14..0x18].copy_from_slice(&0x0u32.to_le_bytes());
    data[0x18..0x1c].copy_from_slice(&0x1000u32.to_le_bytes());

    // Ro: file offset 0x1100, memory offset 0x1000, decompressed size 0x1000
    data[0x20..0x24].copy_from_slice(&0x1100u32.to_le_bytes());
    data[0x24..0x28].copy_from_slice(&0x1000u32.to_le_bytes());
    data[0x28..0x2c].copy_from_slice(&0x1000u32.to_le_bytes());

    // Data: file offset 0x2100, memory offset 0x2000, decompressed size 0x1000
    data[0x30..0x34].copy_from_slice(&0x2100u32.to_le_bytes());
    data[0x34..0x38].copy_from_slice(&0x2000u32.to_le_bytes());
    data[0x38..0x3c].copy_from_slice(&0x1000u32.to_le_bytes());

    data
}

fn build_test_npdm() -> Vec<u8> {
    let mut npdm = vec![0u8; NPDM_HEADER_SIZE];
    npdm[0..4].copy_from_slice(NPDM_MAGIC);
    npdm[0x10..0x1a].copy_from_slice(b"TEST-GAME\0");
    npdm[0x20..0x28].copy_from_slice(&0x0100_1234_5678_0000u64.to_le_bytes());
    npdm
}

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
        file_entries.extend_from_slice(&0u32.to_le_bytes());

        current_data_offset += data.len() as u64;
    }

    header.extend_from_slice(&(string_table.len() as u32).to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());

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
fn test_exefs_program_loading() {
    let nso_bytes = build_test_nso();
    let npdm_bytes = build_test_npdm();

    let files = [
        ("main.npdm", npdm_bytes.as_slice()),
        ("main.nso", nso_bytes.as_slice()),
    ];

    let exefs_bytes = build_pfs0(&files);
    let exefs = ExeFsProgram::parse(&exefs_bytes).expect("parse valid ExeFS");

    assert!(exefs.npdm.is_some());
    let npdm = exefs.npdm.as_ref().unwrap();
    assert_eq!(npdm.product_code, "TEST-GAME");
    assert_eq!(npdm.title_id, 0x0100_1234_5678_0000);

    assert_eq!(exefs.modules.len(), 1);
    assert_eq!(exefs.modules[0].name, "main");

    let mut mem = emulator_memory::AddressSpace::default();
    let mut program = emulator_loader::NsoProgram::new();
    exefs.load_into(&mut program, &mut mem, 0x8000_0000).unwrap();
    assert_eq!(program.modules().len(), 1);
    assert_eq!(program.modules()[0].name, "main");
}

#[test]
fn test_exefs_parse_finds_pfs0_at_aligned_offset() {
    let nso_bytes = build_test_nso();
    let npdm_bytes = build_test_npdm();

    let files = [
        ("main.npdm", npdm_bytes.as_slice()),
        ("main.nso", nso_bytes.as_slice()),
    ];

    let exefs_bytes = build_pfs0(&files);
    // Prefix with 0x4000 bytes of dummy hash table data
    let mut padded = vec![0xEEu8; 0x4000];
    padded.extend_from_slice(&exefs_bytes);

    let exefs = ExeFsProgram::parse(&padded).expect("parse ExeFS with prefix hash data");
    assert!(exefs.npdm.is_some());
    assert_eq!(exefs.modules.len(), 1);
    assert_eq!(exefs.modules[0].name, "main");
}

#[test]
fn test_exefs_handles_prefixed_and_clean_module_names() {
    let nso_bytes = build_test_nso();
    let npdm_bytes = build_test_npdm();

    let files = [
        ("exefs/main.npdm", npdm_bytes.as_slice()),
        ("./main.nso", nso_bytes.as_slice()),
    ];

    let exefs_bytes = build_pfs0(&files);
    let exefs = ExeFsProgram::parse(&exefs_bytes).expect("parse ExeFS with path prefixes");
    assert!(exefs.npdm.is_some());
    assert_eq!(exefs.modules.len(), 1);
    assert_eq!(exefs.modules[0].name, "main");
}

#[test]
fn test_exefs_error_reports_archive_contents() {
    let files = [
        ("NintendoLogo.png", [0x89, 0x50, 0x4E, 0x47].as_slice()),
        ("StartupMovie.gif", [0x47, 0x49, 0x46, 0x38].as_slice()),
    ];

    let exefs_bytes = build_pfs0(&files);
    let err = ExeFsProgram::parse(&exefs_bytes).unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("ExeFS contains no valid NSO modules"));
    assert!(err_str.contains("NintendoLogo.png"));
    assert!(err_str.contains("StartupMovie.gif"));
}
