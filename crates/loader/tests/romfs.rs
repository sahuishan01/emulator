use emulator_loader::nca::{
    NcaFsHeader, NcaFsType, NcaHashType, NCA_FS_HEADER_SIZE,
};
use emulator_loader::romfs::{build_romfs_image, RomFsArchive, IVFC_MAGIC};

#[test]
fn test_romfs_archive_directory_hierarchy_and_streaming() {
    let files = [
        ("content/pokemon.pkm", b"Pikachu-025".as_slice()),
        ("content/map/lumiose.bin", b"LumioseCity3DGrid".as_slice()),
        ("system/version.txt", b"v1.0.0".as_slice()),
    ];

    let romfs_data = build_romfs_image(&files);
    let archive = RomFsArchive::parse(&romfs_data).expect("romfs parse should succeed");

    assert!(archive.contains_file("content/pokemon.pkm"));
    assert!(archive.contains_file("romfs:/content/map/lumiose.bin"));
    assert!(archive.contains_file("/system/version.txt"));
    assert!(!archive.contains_file("nonexistent.txt"));

    assert_eq!(archive.get_file("content/pokemon.pkm").unwrap(), b"Pikachu-025");
    assert_eq!(
        archive.get_file("content/map/lumiose.bin").unwrap(),
        b"LumioseCity3DGrid"
    );
    assert_eq!(archive.get_file("system/version.txt").unwrap(), b"v1.0.0");
    assert_eq!(archive.file_size("system/version.txt").unwrap(), 6);
}

#[test]
fn test_romfs_ivfc_section_parsing() {
    let files = [
        ("data/test.bin", b"TrinityEngineDataPayload".as_slice()),
    ];
    let romfs_raw = build_romfs_image(&files);

    // Construct a section payload with IVFC layout:
    // Prefix with dummy data, placing RomFS data at level 5 offset 0x400
    let data_offset = 0x400u64;
    let data_size = romfs_raw.len() as u64;

    let mut section_payload = vec![0u8; data_offset as usize];
    section_payload.extend_from_slice(&romfs_raw);

    // Build NcaFsHeader with IVFC descriptor
    let mut fs_header_bytes = [0u8; NCA_FS_HEADER_SIZE];
    fs_header_bytes[0..2].copy_from_slice(&2u16.to_le_bytes()); // version 2
    fs_header_bytes[2] = NcaFsType::RomFs as u8;
    fs_header_bytes[3] = NcaHashType::HierarchicalIntegrity as u8;
    fs_header_bytes[4] = 2; // None encryption for test payload

    // IVFC in hash_data (offset 0x08 in header):
    let hash_data = &mut fs_header_bytes[0x08..0x100];
    hash_data[0..4].copy_from_slice(IVFC_MAGIC);
    hash_data[4..8].copy_from_slice(&0x00020000u32.to_le_bytes()); // version
    hash_data[8..12].copy_from_slice(&0x20u32.to_le_bytes()); // master hash size

    // Level 5 at offset 0x0C + 5 * 24 = 0x0C + 120 = 0x84 = 132
    let l5_off = 0x0C + 5 * 24;
    hash_data[l5_off..l5_off + 8].copy_from_slice(&data_offset.to_le_bytes());
    hash_data[l5_off + 8..l5_off + 16].copy_from_slice(&data_size.to_le_bytes());
    hash_data[l5_off + 16..l5_off + 20].copy_from_slice(&9u32.to_le_bytes()); // block size power

    let parsed_fs_header = NcaFsHeader::parse(&fs_header_bytes).expect("parse fs header");
    let romfs = RomFsArchive::from_section_payload(&section_payload, &parsed_fs_header)
        .expect("from_section_payload should parse IVFC correctly");

    assert!(romfs.contains_file("data/test.bin"));
    assert_eq!(
        romfs.get_file("data/test.bin").unwrap(),
        b"TrinityEngineDataPayload"
    );
}
