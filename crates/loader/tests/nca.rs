use emulator_crypto::{aes128::aes128_xts_encrypt, keys::KeyFile};
use emulator_loader::nca::{
    decrypt_nca3_header, NcaContentType, NcaDistributionType, NcaHeader, NCA_ENCRYPTED_HEADER_SIZE,
    NCA_FS_HEADER_SIZE, NCA_HEADER_SIZE, NCA_MEDIA_UNIT_SIZE,
};

const ARCHIVE_SIZE: u64 = 0x2000;

fn valid_header() -> Vec<u8> {
    let mut header = vec![0; NCA_HEADER_SIZE];
    header[0x200..0x204].copy_from_slice(b"NCA3");
    header[0x204] = NcaDistributionType::Download as u8;
    header[0x205] = NcaContentType::Program as u8;
    header[0x206] = 2;
    header[0x207] = 1;
    header[0x208..0x210].copy_from_slice(&ARCHIVE_SIZE.to_le_bytes());
    header[0x210..0x218].copy_from_slice(&0x0100_0000_0000_0001u64.to_le_bytes());
    header[0x218..0x21c].copy_from_slice(&7u32.to_le_bytes());
    header[0x21c..0x220].copy_from_slice(&0x000b_0000u32.to_le_bytes());
    header[0x220] = 0x17;
    header[0x221] = 3;
    header[0x230..0x240].copy_from_slice(&[0xA5; 16]);

    // Section 0 covers media units [6, 12), i.e. byte offsets [0xC00, 0x1800).
    header[0x240..0x244].copy_from_slice(&6u32.to_le_bytes());
    header[0x244..0x248].copy_from_slice(&12u32.to_le_bytes());
    header[0x280..0x2a0].copy_from_slice(&[0x5A; 32]);
    header
}

#[test]
fn parses_nca3_metadata_sections_and_header_hash() {
    let parsed = NcaHeader::parse_decrypted(&valid_header(), ARCHIVE_SIZE).unwrap();
    assert_eq!(parsed.distribution_type, NcaDistributionType::Download);
    assert_eq!(parsed.content_type, NcaContentType::Program);
    assert_eq!(parsed.key_generation_old, 2);
    assert_eq!(parsed.key_area_encryption_key_index, 1);
    assert_eq!(parsed.content_size, ARCHIVE_SIZE);
    assert_eq!(parsed.program_id, 0x0100_0000_0000_0001);
    assert_eq!(parsed.content_index, 7);
    assert_eq!(parsed.sdk_addon_version, 0x000b_0000);
    assert_eq!(parsed.key_generation, 0x17);
    assert_eq!(parsed.signature_key_generation, 3);
    assert_eq!(parsed.rights_id, [0xA5; 16]);
    let section = parsed.sections[0].unwrap();
    assert_eq!((section.start, section.end), (0xC00, 0x1800));
    assert_eq!(section.fs_header_hash, [0x5A; 32]);
    assert!(parsed.sections[1..].iter().all(Option::is_none));
}

#[test]
fn accepts_all_content_types_and_distribution_values() {
    for content_type in 0..=5 {
        let mut bytes = valid_header();
        bytes[0x205] = content_type;
        assert!(NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_ok());
    }
    for distribution in 0..=1 {
        let mut bytes = valid_header();
        bytes[0x204] = distribution;
        assert!(NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_ok());
    }
}

#[test]
fn every_truncated_decrypted_header_prefix_is_rejected() {
    let bytes = valid_header();
    for end in 0..NCA_HEADER_SIZE {
        assert!(
            NcaHeader::parse_decrypted(&bytes[..end], ARCHIVE_SIZE).is_err(),
            "prefix {end}"
        );
    }
}

#[test]
fn rejects_unsupported_magic_including_earlier_nca_generations() {
    for magic in [b"NOPE", b"NCA0", b"NCA1", b"NCA2"] {
        let mut bytes = valid_header();
        bytes[0x200..0x204].copy_from_slice(magic);
        assert!(NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_err());
    }
}

#[test]
fn rejects_reserved_enum_values_and_key_indices() {
    for (offset, value) in [(0x204, 2), (0x205, 6), (0x206, 3), (0x207, 3)] {
        let mut bytes = valid_header();
        bytes[offset] = value;
        assert!(
            NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_err(),
            "offset {offset:#x}"
        );
    }
}

#[test]
fn content_size_must_be_aligned_and_equal_to_archive_length() {
    for (declared, actual) in [(0x1fffu64, 0x1fffu64), (0x2000, 0x1e00), (0xA00, 0xA00)] {
        let mut bytes = valid_header();
        bytes[0x208..0x210].copy_from_slice(&declared.to_le_bytes());
        assert!(NcaHeader::parse_decrypted(&bytes, actual).is_err());
    }
}

#[test]
fn rejects_nonzero_reserved_main_header_bytes() {
    let mut bytes = valid_header();
    bytes[0x22f] = 1;
    assert!(NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_err());
}

#[test]
fn rejects_empty_reversed_overlapping_and_out_of_bounds_sections() {
    let cases = [
        (0u32, 0u32), // inactive with a nonzero hash
        (6, 6),       // empty
        (12, 6),      // reversed
        (5, 8),       // overlaps the encrypted header
        (6, 17),      // extends past archive end
    ];
    for (start, end) in cases {
        let mut bytes = valid_header();
        bytes[0x240..0x244].copy_from_slice(&start.to_le_bytes());
        bytes[0x244..0x248].copy_from_slice(&end.to_le_bytes());
        if start == 0 && end == 0 {
            bytes[0x280] = 1;
        }
        assert!(
            NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_err(),
            "range {start}..{end}"
        );
    }

    let mut overlap = valid_header();
    overlap[0x250..0x254].copy_from_slice(&11u32.to_le_bytes());
    overlap[0x254..0x258].copy_from_slice(&13u32.to_le_bytes());
    overlap[0x2a0..0x2c0].copy_from_slice(&[0x11; 32]);
    assert!(NcaHeader::parse_decrypted(&overlap, ARCHIVE_SIZE).is_err());
}

#[test]
fn section_indices_are_preserved_without_assuming_physical_order() {
    let mut bytes = valid_header();
    // Entry 0 points later in the file; entry 1 remains a distinct earlier section.
    bytes[0x240..0x244].copy_from_slice(&14u32.to_le_bytes());
    bytes[0x244..0x248].copy_from_slice(&16u32.to_le_bytes());
    bytes[0x250..0x254].copy_from_slice(&6u32.to_le_bytes());
    bytes[0x254..0x258].copy_from_slice(&12u32.to_le_bytes());
    bytes[0x2a0..0x2c0].copy_from_slice(&[0x11; 32]);

    let parsed = NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).unwrap();
    assert_eq!(
        (
            parsed.sections[0].unwrap().start,
            parsed.sections[0].unwrap().end
        ),
        (0x1c00, 0x2000)
    );
    assert_eq!(
        (
            parsed.sections[1].unwrap().start,
            parsed.sections[1].unwrap().end
        ),
        (0x0c00, 0x1800)
    );
}

#[test]
fn rejects_nonzero_section_reserved_bytes_and_short_plaintext_buffers() {
    let mut bytes = valid_header();
    bytes[0x24f] = 1;
    assert!(NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_err());

    let mut bytes = valid_header();
    bytes.truncate(NCA_HEADER_SIZE - 1);
    assert!(NcaHeader::parse_decrypted(&bytes, ARCHIVE_SIZE).is_err());
}

#[test]
fn decrypts_all_header_sectors_with_big_endian_sector_tweaks() {
    let key = [0x11; 32];
    let key_text = format!("header_key = {}", "11".repeat(32));
    let keys = KeyFile::parse(key_text.as_bytes()).unwrap();
    let mut plaintext = vec![0; NCA_ENCRYPTED_HEADER_SIZE];
    plaintext[..NCA_HEADER_SIZE].copy_from_slice(&valid_header());
    for index in 0..4 {
        let start = NCA_HEADER_SIZE + index * NCA_FS_HEADER_SIZE;
        plaintext[start..start + NCA_FS_HEADER_SIZE].fill((index + 1) as u8);
    }

    let mut encrypted = plaintext.clone();
    for sector_index in 0..NCA_ENCRYPTED_HEADER_SIZE / NCA_MEDIA_UNIT_SIZE as usize {
        let start = sector_index * NCA_MEDIA_UNIT_SIZE as usize;
        let end = start + NCA_MEDIA_UNIT_SIZE as usize;
        aes128_xts_encrypt(
            &key,
            (sector_index as u128).to_be_bytes(),
            &mut encrypted[start..end],
        )
        .unwrap();
    }
    encrypted.resize(ARCHIVE_SIZE as usize, 0xCC);

    let decrypted = decrypt_nca3_header(&encrypted, &keys).unwrap();
    assert_eq!(decrypted.metadata.program_id, 0x0100_0000_0000_0001);
    for (index, fs_header) in decrypted.fs_headers.iter().enumerate() {
        assert!(fs_header.iter().all(|byte| *byte == (index + 1) as u8));
    }
}

#[test]
fn encrypted_header_rejects_truncation_or_missing_header_key() {
    let keys = KeyFile::parse(b"other_key = 00112233445566778899aabbccddeeff").unwrap();
    assert!(decrypt_nca3_header(&[0; NCA_ENCRYPTED_HEADER_SIZE - 1], &keys).is_err());
    assert!(decrypt_nca3_header(&[0; NCA_ENCRYPTED_HEADER_SIZE], &keys).is_err());
}

#[test]
fn test_make_section_ctr_format() {
    use emulator_loader::nca::make_section_ctr;

    let section_ctr = [1, 2, 3, 4, 5, 6, 7, 8];
    let offset = 0x1000u64; // block index 0x100
    let ctr = make_section_ctr(&section_ctr, offset);

    // Upper 8 bytes are reversed section_ctr
    assert_eq!(&ctr[0..8], &[8, 7, 6, 5, 4, 3, 2, 1]);
    // Lower 8 bytes are big-endian block index
    assert_eq!(&ctr[8..16], &(0x100u64).to_be_bytes());
}

#[test]
fn test_verify_fs_header_hashes_and_mismatch() {
    use emulator_crypto::sha256::Sha256;
    use emulator_loader::nca::NcaDecryptedHeader;

    let mut fs_headers = [[0u8; NCA_FS_HEADER_SIZE]; 4];
    fs_headers[0][0..2].copy_from_slice(&2u16.to_le_bytes()); // version 2
    fs_headers[0][2] = 0; // RomFs
    fs_headers[0][3] = 3; // IVFC
    fs_headers[0][4] = 4; // CTR
    fs_headers[0][0x140..0x148].copy_from_slice(&[0x12; 8]);

    let fs_hash = Sha256::digest(&fs_headers[0]);
    let mut header_bytes = valid_header();
    header_bytes[0x280..0x2a0].copy_from_slice(&fs_hash);

    let metadata = NcaHeader::parse_decrypted(&header_bytes, ARCHIVE_SIZE).unwrap();
    let dec_header = NcaDecryptedHeader {
        metadata,
        fs_headers,
    };

    assert!(dec_header.verify_fs_header_hashes().is_ok());

    // Corrupt one byte of fs_header
    let mut corrupt_fs = fs_headers;
    corrupt_fs[0][0x50] ^= 0xFF;
    let corrupt_dec = NcaDecryptedHeader {
        metadata: dec_header.metadata.clone(),
        fs_headers: corrupt_fs,
    };
    assert!(corrupt_dec.verify_fs_header_hashes().is_err());
}

#[test]
fn test_decrypt_section_ctr_with_title_key() {
    use emulator_crypto::aes128::aes128_ctr_xor;
    use emulator_crypto::sha256::Sha256;
    use emulator_loader::nca::{make_section_ctr, NcaDecryptedHeader};

    let title_key = [0x77u8; 16];
    let rights_id = [0xA5u8; 16];
    let key_text = format!(
        "titlekey_a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5 = {}",
        "77".repeat(16)
    );
    let keys = KeyFile::parse(key_text.as_bytes()).unwrap();

    let mut fs_headers = [[0u8; NCA_FS_HEADER_SIZE]; 4];
    fs_headers[0][0..2].copy_from_slice(&2u16.to_le_bytes()); // version 2
    fs_headers[0][2] = 0; // RomFs
    fs_headers[0][3] = 3; // IVFC
    fs_headers[0][4] = 4; // CTR
    fs_headers[0][0x140..0x148].copy_from_slice(&[0x42; 8]);

    let fs_hash = Sha256::digest(&fs_headers[0]);
    let mut header_bytes = valid_header();
    header_bytes[0x230..0x240].copy_from_slice(&rights_id);
    header_bytes[0x280..0x2a0].copy_from_slice(&fs_hash);

    let metadata = NcaHeader::parse_decrypted(&header_bytes, ARCHIVE_SIZE).unwrap();
    let dec_header = NcaDecryptedHeader {
        metadata,
        fs_headers,
    };

    let original_payload = vec![0x33u8; 0xC00]; // 0xC00 bytes section
    let mut encrypted_payload = original_payload.clone();
    let section_start = 0xC00u64;
    let initial_counter = make_section_ctr(&[0x42; 8], section_start);
    aes128_ctr_xor(&title_key, initial_counter, &mut encrypted_payload).unwrap();

    let mut archive = vec![0u8; ARCHIVE_SIZE as usize];
    archive[section_start as usize..section_start as usize + 0xC00]
        .copy_from_slice(&encrypted_payload);

    let decrypted = dec_header.decrypt_section(&archive, 0, &keys).unwrap();
    assert_eq!(decrypted, original_payload);
}

#[test]
fn test_decrypt_section_ctr_with_key_area() {
    use emulator_crypto::aes128::{aes128_ctr_xor, Aes128};
    use emulator_crypto::sha256::Sha256;
    use emulator_loader::nca::{make_section_ctr, NcaDecryptedHeader};

    let kaek = [0x88u8; 16];
    let raw_content_key = [0x99u8; 16];
    let mut encrypted_key_entry = raw_content_key;
    // Encrypt content key with kaek using AES-128-ECB
    let aes = Aes128::new(&kaek);
    aes.encrypt_block(&mut encrypted_key_entry);

    let key_text = format!("key_area_key_application_00 = {}", "88".repeat(16));
    let keys = KeyFile::parse(key_text.as_bytes()).unwrap();

    let mut fs_headers = [[0u8; NCA_FS_HEADER_SIZE]; 4];
    fs_headers[0][0..2].copy_from_slice(&2u16.to_le_bytes()); // version 2
    fs_headers[0][2] = 0; // RomFs
    fs_headers[0][3] = 3; // IVFC
    fs_headers[0][4] = 4; // CTR
    fs_headers[0][0x140..0x148].copy_from_slice(&[0x55; 8]);

    let fs_hash = Sha256::digest(&fs_headers[0]);
    let mut header_bytes = valid_header();
    header_bytes[0x206] = 0; // Legacy key generation 0
    header_bytes[0x207] = 0; // Application key area
    header_bytes[0x220] = 1; // MasterKey 0
    header_bytes[0x230..0x240].fill(0); // No rights ID (use Key Area)
    header_bytes[0x280..0x2a0].copy_from_slice(&fs_hash);
    // Key area slot 2 at 0x300 + 2 * 16 = 0x320
    header_bytes[0x320..0x330].copy_from_slice(&encrypted_key_entry);

    let metadata = NcaHeader::parse_decrypted(&header_bytes, ARCHIVE_SIZE).unwrap();
    let dec_header = NcaDecryptedHeader {
        metadata,
        fs_headers,
    };

    let original_payload = vec![0x44u8; 0xC00];
    let mut encrypted_payload = original_payload.clone();
    let section_start = 0xC00u64;
    let initial_counter = make_section_ctr(&[0x55; 8], section_start);
    aes128_ctr_xor(&raw_content_key, initial_counter, &mut encrypted_payload).unwrap();

    let mut archive = vec![0u8; ARCHIVE_SIZE as usize];
    archive[section_start as usize..section_start as usize + 0xC00]
        .copy_from_slice(&encrypted_payload);

    let decrypted = dec_header.decrypt_section(&archive, 0, &keys).unwrap();
    assert_eq!(decrypted, original_payload);
}

