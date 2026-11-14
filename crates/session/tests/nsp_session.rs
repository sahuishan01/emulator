use emulator_crypto::aes128::{aes128_ctr_xor, aes128_ecb_encrypt, aes128_xts_encrypt};
use emulator_crypto::keys::KeyFile;
use emulator_crypto::sha256::Sha256;
use emulator_loader::cnmt::{CnmtContentType, ContentMetaType};
use emulator_loader::nca::{
    make_section_ctr, NcaEncryptionType, NcaFsType, NcaHashType, NCA_ENCRYPTED_HEADER_SIZE,
    NCA_FS_HEADER_SIZE, NCA_HEADER_SIZE,
};
use emulator_loader::nsp::NspArchive;
use emulator_loader::romfs::build_romfs_image;
use emulator_session::{RunOutcome, Session};

fn build_minimal_nso(instructions: &[u32]) -> Vec<u8> {
    let mut text_payload = Vec::new();
    for &insn in instructions {
        text_payload.extend_from_slice(&insn.to_le_bytes());
    }
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
    nso[0x10..0x14].copy_from_slice(&(text_offset as u32).to_le_bytes());
    nso[0x18..0x1c].copy_from_slice(&(text_payload.len() as u32).to_le_bytes());
    nso[0x1c..0x20].copy_from_slice(&(text_payload.len() as u32).to_le_bytes());
    nso[0x20..0x24].copy_from_slice(&(ro_offset as u32).to_le_bytes());
    nso[0x24..0x28].copy_from_slice(&(text_payload.len() as u32).to_le_bytes());
    nso[0x28..0x2c].copy_from_slice(&(ro_payload.len() as u32).to_le_bytes());
    nso[0x2c..0x30].copy_from_slice(&(ro_payload.len() as u32).to_le_bytes());
    let data_mem_offset = (text_payload.len() + ro_payload.len()) as u32;
    nso[0x30..0x34].copy_from_slice(&(data_offset as u32).to_le_bytes());
    nso[0x34..0x38].copy_from_slice(&data_mem_offset.to_le_bytes());
    nso[0x38..0x3c].copy_from_slice(&(data_payload.len() as u32).to_le_bytes());

    nso[text_offset..ro_offset].copy_from_slice(&text_payload);
    nso[ro_offset..data_offset].copy_from_slice(&ro_payload);
    nso[data_offset..total_file_size].copy_from_slice(&data_payload);
    nso
}

fn build_minimal_npdm() -> Vec<u8> {
    let mut npdm = vec![0u8; 0x80];
    npdm[0..4].copy_from_slice(b"META");
    npdm[0x10..0x18].copy_from_slice(b"POKEMON\0");
    npdm[0x20..0x28].copy_from_slice(&0x0100_0000_0000_1000u64.to_le_bytes());
    npdm[0x28..0x2c].copy_from_slice(&44i32.to_le_bytes());
    npdm[0x2c..0x30].copy_from_slice(&0i32.to_le_bytes());
    npdm[0x34..0x38].copy_from_slice(&0x1_0000u32.to_le_bytes());
    npdm
}

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

fn build_test_ticket(
    rights_id: [u8; 16],
    titlekek: &[u8; 16],
    raw_title_key: [u8; 16],
    master_key_rev: u8,
) -> Vec<u8> {
    let mut enc_key = raw_title_key;
    aes128_ecb_encrypt(titlekek, &mut enc_key).unwrap();

    let mut ticket = Vec::new();
    ticket.extend_from_slice(&0x010004u32.to_be_bytes()); // RSA-2048 SHA-256
    ticket.resize(ticket.len() + 0x100, 0x11); // signature
    ticket.resize(ticket.len() + 0x3C, 0); // padding

    let body_start = ticket.len();
    ticket.resize(body_start + 0x180, 0);
    ticket[body_start..body_start + 7].copy_from_slice(b"Root-CA");
    ticket[body_start + 0x40..body_start + 0x50].copy_from_slice(&enc_key);
    ticket[body_start + 0x140] = 2; // version
    ticket[body_start + 0x141] = 0; // Common
    ticket[body_start + 0x145] = master_key_rev;
    ticket[body_start + 0x150..body_start + 0x158].copy_from_slice(&999u64.to_le_bytes());
    ticket[body_start + 0x160..body_start + 0x170].copy_from_slice(&rights_id);
    ticket
}

fn build_cnmt_binary(title_id: u64, prog_id: [u8; 16], prog_size: u64) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&title_id.to_le_bytes());
    data.extend_from_slice(&0x10000u32.to_le_bytes());
    data.push(ContentMetaType::Application as u8);
    data.push(0);
    data.extend_from_slice(&0u16.to_le_bytes()); // ext_header_size
    data.extend_from_slice(&1u16.to_le_bytes()); // content_count = 1
    data.extend_from_slice(&0u16.to_le_bytes());
    data.push(0);
    data.extend_from_slice(&[0, 0, 0]);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&[0, 0, 0, 0]);

    // Record: Program
    data.resize(data.len() + 32, 0); // hash
    data.extend_from_slice(&prog_id);
    let size_bytes = prog_size.to_le_bytes();
    data.extend_from_slice(&size_bytes[..6]);
    data.push(CnmtContentType::Program as u8);
    data.push(0);
    data
}

#[test]
fn test_session_from_nsp_full_decryption_and_execution() {
    let header_key = [0x5au8; 32];
    let titlekek = [0x7bu8; 16];
    let title_key = [0x33u8; 16];
    let rights_id = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];

    let mut keys = KeyFile::default();
    keys.add_key("header_key", &header_key).unwrap();
    keys.add_key("titlekek_00", &titlekek).unwrap();

    // 1. Build Program NCA content:
    // Guest instructions: MOV X0, #42; SVC #0x07 (ExitProcess)
    let guest_instructions = vec![
        0xd2800540u32, // MOV X0, #42
        0xd40000e1u32, // SVC #0x07 (ExitProcess)
    ];
    let nso_bytes = build_minimal_nso(&guest_instructions);
    let npdm_bytes = build_minimal_npdm();
    let exefs_raw = build_pfs0(&[("main.npdm", &npdm_bytes), ("main.nso", &nso_bytes)]);
    let mut exefs_payload = exefs_raw;
    while exefs_payload.len() % 512 != 0 {
        exefs_payload.push(0);
    }

    let romfs_raw = build_romfs_image(&[
        ("pokemon.txt", b"Gotta Catch Em All!"),
    ]);
    let mut romfs_payload = romfs_raw;
    while romfs_payload.len() % 512 != 0 {
        romfs_payload.push(0);
    }

    let exefs_start = 0xC00u64;
    let exefs_end = exefs_start + exefs_payload.len() as u64;
    let romfs_start = exefs_end;
    let romfs_end = romfs_start + romfs_payload.len() as u64;
    let total_prog_size = romfs_end;

    // Encrypt ExeFS section with title_key via AES-128-CTR
    let exefs_ctr_seed = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let mut exefs_section = exefs_payload.clone();
    let initial_ctr0 = make_section_ctr(&exefs_ctr_seed, exefs_start);
    aes128_ctr_xor(&title_key, initial_ctr0, &mut exefs_section).unwrap();

    // Plaintext RomFS section
    let romfs_section = romfs_payload.clone();

    // Program NCA main header
    let mut prog_main_header = vec![0u8; NCA_HEADER_SIZE];
    prog_main_header[0x200..0x204].copy_from_slice(b"NCA3");
    prog_main_header[0x204] = 0; // Download
    prog_main_header[0x205] = 0; // Program
    prog_main_header[0x208..0x210].copy_from_slice(&total_prog_size.to_le_bytes());
    prog_main_header[0x210..0x218].copy_from_slice(&0x0100_0000_0000_1000u64.to_le_bytes());
    prog_main_header[0x220] = 1; // key_generation
    prog_main_header[0x230..0x240].copy_from_slice(&rights_id);

    // Section 0 (ExeFS)
    let s0_start_block = (exefs_start / 0x200) as u32;
    let s0_end_block = (exefs_end / 0x200) as u32;
    prog_main_header[0x240..0x244].copy_from_slice(&s0_start_block.to_le_bytes());
    prog_main_header[0x244..0x248].copy_from_slice(&s0_end_block.to_le_bytes());

    // Section 1 (RomFS)
    let s1_start_block = (romfs_start / 0x200) as u32;
    let s1_end_block = (romfs_end / 0x200) as u32;
    prog_main_header[0x250..0x254].copy_from_slice(&s1_start_block.to_le_bytes());
    prog_main_header[0x254..0x258].copy_from_slice(&s1_end_block.to_le_bytes());

    // FS headers
    let mut fs_header_0 = vec![0u8; NCA_FS_HEADER_SIZE];
    fs_header_0[0..2].copy_from_slice(&2u16.to_le_bytes());
    fs_header_0[2] = NcaFsType::PartitionFs as u8;
    fs_header_0[3] = NcaHashType::Auto as u8;
    fs_header_0[4] = NcaEncryptionType::AesCtr as u8; // CTR
    fs_header_0[0x140..0x148].copy_from_slice(&exefs_ctr_seed);

    let mut fs_header_1 = vec![0u8; NCA_FS_HEADER_SIZE];
    fs_header_1[0..2].copy_from_slice(&2u16.to_le_bytes());
    fs_header_1[2] = NcaFsType::RomFs as u8;
    fs_header_1[3] = NcaHashType::Auto as u8;
    fs_header_1[4] = NcaEncryptionType::None as u8; // None

    let hash_0 = Sha256::digest(&fs_header_0);
    let hash_1 = Sha256::digest(&fs_header_1);
    prog_main_header[0x280..0x2a0].copy_from_slice(&hash_0);
    prog_main_header[0x2a0..0x2c0].copy_from_slice(&hash_1);

    let mut prog_header_area = vec![0u8; NCA_ENCRYPTED_HEADER_SIZE];
    prog_header_area[0..NCA_HEADER_SIZE].copy_from_slice(&prog_main_header);
    prog_header_area[0x400..0x600].copy_from_slice(&fs_header_0);
    prog_header_area[0x600..0x800].copy_from_slice(&fs_header_1);

    // Encrypt Program NCA header using header_key
    let mut enc_prog_header = prog_header_area.clone();
    for sector_index in 0..NCA_ENCRYPTED_HEADER_SIZE / 0x200 {
        let start = sector_index * 0x200;
        let end = start + 0x200;
        let tweak = (sector_index as u128).to_be_bytes();
        aes128_xts_encrypt(&header_key, tweak, &mut enc_prog_header[start..end]).unwrap();
    }

    let mut prog_nca = Vec::new();
    prog_nca.extend_from_slice(&enc_prog_header);
    prog_nca.extend_from_slice(&exefs_section);
    prog_nca.extend_from_slice(&romfs_section);
    assert_eq!(prog_nca.len() as u64, total_prog_size);

    // 2. Build Meta NCA (.cnmt.nca)
    let prog_id = [
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88,
    ];
    let cnmt_bytes = build_cnmt_binary(0x0100_0000_0000_1000, prog_id, prog_nca.len() as u64);
    let cnmt_pfs0 = build_pfs0(&[("Application_0100000000001000.cnmt", &cnmt_bytes)]);
    let mut cnmt_pfs0_padded = cnmt_pfs0;
    while cnmt_pfs0_padded.len() % 512 != 0 {
        cnmt_pfs0_padded.push(0);
    }

    let meta_total_size = 0xC00 + cnmt_pfs0_padded.len() as u64;
    let mut meta_main_header = vec![0u8; NCA_HEADER_SIZE];
    meta_main_header[0x200..0x204].copy_from_slice(b"NCA3");
    meta_main_header[0x204] = 0;
    meta_main_header[0x205] = 1; // Meta
    meta_main_header[0x208..0x210].copy_from_slice(&meta_total_size.to_le_bytes());
    meta_main_header[0x240..0x244].copy_from_slice(&((0xC00 / 0x200) as u32).to_le_bytes());
    meta_main_header[0x244..0x248].copy_from_slice(&((meta_total_size / 0x200) as u32).to_le_bytes());

    let mut meta_fs_0 = vec![0u8; NCA_FS_HEADER_SIZE];
    meta_fs_0[0..2].copy_from_slice(&2u16.to_le_bytes());
    meta_fs_0[2] = NcaFsType::PartitionFs as u8;
    meta_fs_0[3] = NcaHashType::Auto as u8;
    meta_fs_0[4] = NcaEncryptionType::None as u8;

    let meta_hash_0 = Sha256::digest(&meta_fs_0);
    meta_main_header[0x280..0x2a0].copy_from_slice(&meta_hash_0);

    let mut meta_header_area = vec![0u8; NCA_ENCRYPTED_HEADER_SIZE];
    meta_header_area[0..NCA_HEADER_SIZE].copy_from_slice(&meta_main_header);
    meta_header_area[0x400..0x600].copy_from_slice(&meta_fs_0);

    let mut enc_meta_header = meta_header_area.clone();
    for sector_index in 0..NCA_ENCRYPTED_HEADER_SIZE / 0x200 {
        let start = sector_index * 0x200;
        let end = start + 0x200;
        let tweak = (sector_index as u128).to_be_bytes();
        aes128_xts_encrypt(&header_key, tweak, &mut enc_meta_header[start..end]).unwrap();
    }

    let mut meta_nca = Vec::new();
    meta_nca.extend_from_slice(&enc_meta_header);
    meta_nca.extend_from_slice(&cnmt_pfs0_padded);

    // 3. Build Ticket (.tik)
    let ticket_bytes = build_test_ticket(rights_id, &titlekek, title_key, 0);

    // 4. Build NSP (PFS0 containing Ticket, Meta NCA, Program NCA)
    let prog_filename = format!("123456789abcdef01122334455667788.nca");
    let nsp_bytes = build_pfs0(&[
        ("title.tik", &ticket_bytes),
        ("00000000000000000000000000000001.cnmt.nca", &meta_nca),
        (&prog_filename, &prog_nca),
    ]);

    // Verify NspArchive directly
    let nsp = NspArchive::parse(&nsp_bytes).expect("NSP must parse");
    assert_eq!(nsp.parse_tickets().len(), 1);
    let cnmt = nsp.parse_cnmt(&keys).expect("CNMT must parse").expect("CNMT must exist");
    assert_eq!(cnmt.title_id, 0x0100_0000_0000_1000);
    assert_eq!(cnmt.contents.len(), 1);

    // Now test Session::from_nsp!
    let mut session = Session::from_nsp(&nsp_bytes, &mut keys)
        .expect("Session::from_nsp must succeed and load guest program");

    // Verify that the title key was decrypted from the ticket and imported into keys
    assert_eq!(keys.title_key(&rights_id), Some(&title_key));

    // Execute the guest instructions
    let outcome = session.run(100);
    assert_eq!(outcome.unwrap(), RunOutcome::Exited);

    // Verify CPU state: X0 was set to 42 by guest MOV X0, #42
    assert_eq!(session.cpu().x[0], 42);
    assert!(session.instructions() >= 2);
}
