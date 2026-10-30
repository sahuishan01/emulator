use emulator_loader::{LoadError, NroImage};
use emulator_memory::{AddressSpace, Permissions};

const HELLO: &[u8] = include_bytes!("../../../examples/homebrew/hello.nro");

fn set_word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn valid_nro_maps_segment_permissions_and_zero_bss() {
    let image = NroImage::parse(HELLO).unwrap();
    let mut memory = AddressSpace::default();
    let loaded = image.map_into(&mut memory, 0x100000).unwrap();
    assert_eq!(loaded.entry, 0x100000);
    assert_eq!(loaded.end, 0x104000);
    assert_eq!(
        memory.query(0x100000).unwrap().permissions,
        Permissions::READ_EXECUTE
    );
    assert_eq!(
        memory.query(0x101000).unwrap().permissions,
        Permissions::READ
    );
    assert_eq!(
        memory.query(0x102000).unwrap().permissions,
        Permissions::READ_WRITE
    );
    assert_eq!(memory.read_u64(0x102000).unwrap(), 0x0123456789abcdef);
    assert_eq!(memory.read_u64(0x103000).unwrap(), 0);
    assert!(memory.write_u32(0x100000, 0).is_err());
    assert!(memory.fetch_u32(0x101000).is_err());
}

#[test]
fn every_truncated_prefix_fails_without_panicking() {
    for length in 0..HELLO.len() {
        assert!(
            NroImage::parse(&HELLO[..length]).is_err(),
            "prefix {length}"
        );
    }
}

#[test]
fn malformed_segments_and_unsupported_variants_are_rejected() {
    for (offset, value) in [
        (0x14, 1),
        (0x1c, 2),
        (4, 0x100),
        (0x20, 0x1000),
        (0x24, 0),
        (0x28, 0),
        (0x2c, 1),
        (0x30, 0xfffff000),
        (0x38, u32::MAX),
        (0x68, 1),
    ] {
        let mut bytes = HELLO.to_vec();
        set_word(&mut bytes, offset, value);
        assert!(
            NroImage::parse(&bytes).is_err(),
            "accepted field {offset:#x}={value:#x}"
        );
    }
    let mut bytes = HELLO.to_vec();
    bytes[0x10] = b'X';
    assert!(NroImage::parse(&bytes).is_err());
}

#[test]
fn mapping_conflict_is_atomic_and_does_not_replace_existing_data() {
    let image = NroImage::parse(HELLO).unwrap();
    let mut memory = AddressSpace::default();
    memory
        .map(0x101000, vec![0xaa; 4096], Permissions::READ)
        .unwrap();
    assert!(image.map_into(&mut memory, 0x100000).is_err());
    assert!(memory.query(0x100000).is_none());
    assert!(memory.query(0x102000).is_none());
    assert_eq!(memory.read_u32(0x101000).unwrap(), 0xaaaaaaaa);
}

#[test]
fn load_base_overflow_and_resident_limit_fail_before_publish() {
    let image = NroImage::parse(HELLO).unwrap();
    let mut memory = AddressSpace::with_limit(1);
    assert!(image.map_into(&mut memory, 0x100000).is_err());
    assert_eq!(memory.mapped_bytes(), 0);
    assert!(matches!(
        image.map_into(&mut memory, u64::MAX - 4095),
        Err(LoadError::AddressOverflow)
    ));
    assert_eq!(memory.mapped_bytes(), 0);
}
