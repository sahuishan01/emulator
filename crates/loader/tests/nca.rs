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
