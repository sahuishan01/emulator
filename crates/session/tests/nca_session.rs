use emulator_crypto::keys::KeyFile;
use emulator_crypto::sha256::Sha256;
use emulator_loader::nca::{
    NcaFsType, NcaHashType, NCA_ENCRYPTED_HEADER_SIZE, NCA_FS_HEADER_SIZE, NCA_HEADER_SIZE,
};
use emulator_loader::romfs::build_romfs_image;
use emulator_session::{RunOutcome, Session};

/// Builds a minimal synthetic uncompressed NSO0 module.
fn build_minimal_nso(instructions: &[u32]) -> Vec<u8> {
    let mut text_payload = Vec::new();
    for &insn in instructions {
        text_payload.extend_from_slice(&insn.to_le_bytes());
    }
    // Pad to 4096 bytes
    while text_payload.len() % 4096 != 0 {
        text_payload.extend_from_slice(&0xd503201fu32.to_le_bytes()); // NOP
    }

    let ro_payload = vec![0u8; 4096];
    let data_payload = vec![0u8; 4096];

    let header_size = 0x100usize;
    let text_offset = header_size;
    let ro_offset = text_offset + text_payload.len();
    let data_offset = ro_offset + ro_payload.len();
    let total_file_size = data_offset + data_payload.len();

    let mut nso = vec![0u8; total_file_size];
    nso[0..4].copy_from_slice(b"NSO0");
    nso[4..8].copy_from_slice(&0u32.to_le_bytes()); // version
    nso[8..12].copy_from_slice(&0u32.to_le_bytes()); // flags (uncompressed, no hash check)

    // Text section header (offset 0x10)
    nso[0x10..0x14].copy_from_slice(&(text_offset as u32).to_le_bytes());
    nso[0x14..0x18].copy_from_slice(&0u32.to_le_bytes()); // memory offset 0
    nso[0x18..0x1c].copy_from_slice(&(text_payload.len() as u32).to_le_bytes());
    nso[0x1c..0x20].copy_from_slice(&(text_payload.len() as u32).to_le_bytes());

    // Ro section header (offset 0x20)
    nso[0x20..0x24].copy_from_slice(&(ro_offset as u32).to_le_bytes());
    nso[0x24..0x28].copy_from_slice(&(text_payload.len() as u32).to_le_bytes());
    nso[0x28..0x2c].copy_from_slice(&(ro_payload.len() as u32).to_le_bytes());
    nso[0x2c..0x30].copy_from_slice(&(ro_payload.len() as u32).to_le_bytes());

    // Data section header (offset 0x30)
    let data_mem_offset = (text_payload.len() + ro_payload.len()) as u32;
    nso[0x30..0x34].copy_from_slice(&(data_offset as u32).to_le_bytes());
    nso[0x34..0x38].copy_from_slice(&data_mem_offset.to_le_bytes());
    nso[0x38..0x3c].copy_from_slice(&(data_payload.len() as u32).to_le_bytes());
    nso[0x3c..0x40].copy_from_slice(&0u32.to_le_bytes()); // bss_size 0

    // Copy payloads
    nso[text_offset..ro_offset].copy_from_slice(&text_payload);
    nso[ro_offset..data_offset].copy_from_slice(&ro_payload);
    nso[data_offset..total_file_size].copy_from_slice(&data_payload);

    nso
}

/// Builds a minimal synthetic NPDM metadata.
fn build_minimal_npdm() -> Vec<u8> {
    let mut npdm = vec![0u8; 0x80];
    npdm[0..4].copy_from_slice(b"META");
    // Product code at 0x10..0x20
    npdm[0x10..0x18].copy_from_slice(b"POKEMON\0");
    // Title ID at 0x20..0x28
    npdm[0x20..0x28].copy_from_slice(&0x0100_0000_0000_1000u64.to_le_bytes());
    // Main thread priority at 0x28..0x2c
    npdm[0x28..0x2c].copy_from_slice(&44i32.to_le_bytes());
    // Default CPU core at 0x2c..0x30
    npdm[0x2c..0x30].copy_from_slice(&0i32.to_le_bytes());
    // Main thread stack size at 0x34..0x38
    npdm[0x34..0x38].copy_from_slice(&0x1_0000u32.to_le_bytes()); // 64 KiB
    npdm
}

/// Builds an ExeFS PFS0 partition archive from files.
fn build_pfs0(files: &[(&str, &[u8])]) -> Vec<u8> {
    let num_files = files.len() as u32;
    let mut string_table = Vec::new();
    let mut file_entries = Vec::new();
    let mut current_offset = 0u64;

    for (name, content) in files {
        let name_offset = string_table.len() as u32;
        string_table.extend_from_slice(name.as_bytes());
        string_table.push(0); // null terminator

        let file_offset = current_offset;
        let file_size = content.len() as u64;
        current_offset += file_size;

        // PFS0 entry: offset: u64, size: u64, string_offset: u32, reserved: u32
        file_entries.extend_from_slice(&file_offset.to_le_bytes());
        file_entries.extend_from_slice(&file_size.to_le_bytes());
        file_entries.extend_from_slice(&name_offset.to_le_bytes());
        file_entries.extend_from_slice(&0u32.to_le_bytes());
    }

    let header_size = 16 + file_entries.len() + string_table.len();
    let mut pfs0 = vec![0u8; header_size];
    pfs0[0..4].copy_from_slice(b"PFS0");
    pfs0[4..8].copy_from_slice(&num_files.to_le_bytes());
    pfs0[8..12].copy_from_slice(&(string_table.len() as u32).to_le_bytes());
    pfs0[12..16].copy_from_slice(&0u32.to_le_bytes());

    pfs0[16..16 + file_entries.len()].copy_from_slice(&file_entries);
    pfs0[16 + file_entries.len()..16 + file_entries.len() + string_table.len()]
        .copy_from_slice(&string_table);

    for (_, content) in files {
        pfs0.extend_from_slice(content);
    }

    pfs0
}

#[test]
fn test_session_from_nca_execution_and_lifecycle() {
    // AArch64 program that initializes floating-point values, performs vector math,
    // and exits via SVC #0x07 (ExitProcess)
    let program_code = [
        // MOVZ X0, #42
        0xd2800540,
        // SCVTF S0, W0 -> S0 = 42.0
        0x1e220000,
        // FCVT D1, S0 -> D1 = 42.0
        0x1e22c001,
        // FADD D2, D1, D1 -> D2 = 84.0: 0x1e612822
        0x1e612822,
        // FCVTZS X3, D2 -> X3 = 84: sf=1, double=1 -> 0x9e780043
        0x9e780043,
        // SVC #0x07 (ExitProcess syscall)
        0xd40000e1,
    ];

    let nso_bytes = build_minimal_nso(&program_code);
    let npdm_bytes = build_minimal_npdm();

    let exefs_pfs0 = build_pfs0(&[
        ("main.npdm", &npdm_bytes),
        ("main.nso", &nso_bytes),
    ]);

    let romfs_raw = build_romfs_image(&[
        ("pokemon/species.bin", b"PokemonLegendsZA_TrinityEngine".as_slice()),
    ]);

    // Align sections to 0x200 media units
    let mut exefs_section = exefs_pfs0;
    while exefs_section.len() % 0x200 != 0 {
        exefs_section.push(0);
    }
    let mut romfs_section = romfs_raw;
    while romfs_section.len() % 0x200 != 0 {
        romfs_section.push(0);
    }

    let exefs_start = 0xC00u64;
    let exefs_end = exefs_start + exefs_section.len() as u64;
    let romfs_start = exefs_end;
    let romfs_end = romfs_start + romfs_section.len() as u64;
    let total_nca_size = romfs_end;

    // Filesystem headers (0x200 each)
    let mut fs_headers = [[0u8; NCA_FS_HEADER_SIZE]; 4];

    // FS Header 0: PartitionFS (ExeFS)
    fs_headers[0][0..2].copy_from_slice(&2u16.to_le_bytes()); // version
    fs_headers[0][2] = NcaFsType::PartitionFs as u8;
    fs_headers[0][3] = NcaHashType::Auto as u8;
    fs_headers[0][4] = 2; // Encryption None

    // FS Header 1: RomFS
    fs_headers[1][0..2].copy_from_slice(&2u16.to_le_bytes());
    fs_headers[1][2] = NcaFsType::RomFs as u8;
    fs_headers[1][3] = NcaHashType::Auto as u8;
    fs_headers[1][4] = 2; // Encryption None

    let hash0 = Sha256::digest(&fs_headers[0]);
    let hash1 = Sha256::digest(&fs_headers[1]);

    // Main NCA header (0x400 bytes)
    let mut main_header = vec![0u8; NCA_HEADER_SIZE];
    main_header[0x200..0x204].copy_from_slice(b"NCA3");
    main_header[0x204] = 0; // Download
    main_header[0x205] = 0; // Program
    main_header[0x208..0x210].copy_from_slice(&total_nca_size.to_le_bytes());
    main_header[0x210..0x218].copy_from_slice(&0x0100_0000_0000_1000u64.to_le_bytes()); // program_id

    // Section 0 entry (offset 0x240..0x250)
    let sec0_media_start = (exefs_start / 0x200) as u32;
    let sec0_media_end = (exefs_end / 0x200) as u32;
    main_header[0x240..0x244].copy_from_slice(&sec0_media_start.to_le_bytes());
    main_header[0x244..0x248].copy_from_slice(&sec0_media_end.to_le_bytes());

    // Section 1 entry (offset 0x250..0x260)
    let sec1_media_start = (romfs_start / 0x200) as u32;
    let sec1_media_end = (romfs_end / 0x200) as u32;
    main_header[0x250..0x254].copy_from_slice(&sec1_media_start.to_le_bytes());
    main_header[0x254..0x258].copy_from_slice(&sec1_media_end.to_le_bytes());

    // Section hashes at 0x280
    main_header[0x280..0x2a0].copy_from_slice(&hash0);
    main_header[0x2a0..0x2c0].copy_from_slice(&hash1);

    // Assemble plaintext header area (0xC00 bytes):
    // 0x000..0x400: main header
    // 0x400..0x600: fs_headers[0]
    // 0x600..0x800: fs_headers[1]
    // 0x800..0xA00: fs_headers[2]
    // 0xA00..0xC00: fs_headers[3]
    let mut header_area = vec![0u8; NCA_ENCRYPTED_HEADER_SIZE];
    header_area[0..NCA_HEADER_SIZE].copy_from_slice(&main_header);
    header_area[0x400..0x600].copy_from_slice(&fs_headers[0]);
    header_area[0x600..0x800].copy_from_slice(&fs_headers[1]);

    // Encrypt header area with XTS using a known test header_key
    let header_key = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
        0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    ];

    let mut encrypted_header_area = header_area.clone();
    for sector in 0..6 {
        let start = sector * 0x200;
        let end = start + 0x200;
        let tweak = (sector as u128).to_be_bytes();
        emulator_crypto::aes128::aes128_xts_encrypt(
            &header_key,
            tweak,
            &mut encrypted_header_area[start..end],
        )
        .expect("xts encrypt header");
    }

    // Assemble full NCA file
    let mut nca_file = Vec::new();
    nca_file.extend_from_slice(&encrypted_header_area);
    nca_file.extend_from_slice(&exefs_section);
    nca_file.extend_from_slice(&romfs_section);
    assert_eq!(nca_file.len() as u64, total_nca_size);

    // Create KeyFile containing the header_key
    let key_file_text = format!(
        "header_key = {}\n",
        header_key
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join("")
    );
    let keys = KeyFile::parse(key_file_text.as_bytes()).expect("parse test keys");

    // Load Session from the NCA file!
    let mut session = Session::from_nca(&nca_file, &keys).expect("Session::from_nca should succeed");

    // Run session
    let outcome = session.run(100);
    assert_eq!(outcome.unwrap(), RunOutcome::Exited);

    // Verify program execution:
    // X0 was 42, D1 was 42.0, D2 was 84.0, X3 was 84!
    assert_eq!(session.cpu().x[0], 42);
    assert_eq!(session.cpu().read_v_f64(1), 42.0);
    assert_eq!(session.cpu().read_v_f64(2), 84.0);
    assert_eq!(session.cpu().x[3], 84);

    println!(
        "Session successfully ran NCA program: instructions={}",
        session.instructions()
    );
}
