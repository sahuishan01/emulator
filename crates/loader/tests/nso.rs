use emulator_loader::NsoImage;
use emulator_memory::{AddressSpace, Permissions};

fn create_sample_nso(bss_size: u32) -> Vec<u8> {
    let mut data = vec![0u8; 0x100 + 0x3000]; // 0x100 header + 3 pages
    data[0..4].copy_from_slice(b"NSO0"); // Magic
    data[4..8].copy_from_slice(&0u32.to_le_bytes()); // Version
    data[8..12].copy_from_slice(&0u32.to_le_bytes()); // Flags: uncompressed

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

    // BSS size
    data[0x3c..0x40].copy_from_slice(&bss_size.to_le_bytes());

    // Build ID / Module ID (32 bytes)
    for i in 0..32 {
        data[0x40 + i] = (i as u8) + 1;
    }

    // Fill some distinctive payloads
    data[0x100..0x108].copy_from_slice(&0x1122334455667788u64.to_le_bytes()); // text
    data[0x1100..0x1108].copy_from_slice(&0xaabbccddeeff0011u64.to_le_bytes()); // rodata
    data[0x2100..0x2108].copy_from_slice(&0x9988776655443322u64.to_le_bytes()); // data

    data
}

#[test]
fn valid_nso_parses_and_maps_with_bss_and_permissions() {
    let nso_bytes = create_sample_nso(0x1000);
    let image = NsoImage::parse(&nso_bytes).unwrap();
    assert_eq!(image.bss_size(), 0x1000);
    assert_eq!(image.segments()[0].permissions, Permissions::READ_EXECUTE);
    assert_eq!(image.segments()[1].permissions, Permissions::READ);
    assert_eq!(image.segments()[2].permissions, Permissions::READ_WRITE);
    assert_eq!(image.segments()[2].memory_size, 0x2000); // 0x1000 data + 0x1000 BSS

    let mut memory = AddressSpace::default();
    let loaded = image.map_into(&mut memory, 0x200000).unwrap();
    assert_eq!(loaded.base, 0x200000);
    assert_eq!(loaded.entry, 0x200000);
    assert_eq!(loaded.end, 0x204000);

    // Text check
    assert_eq!(memory.query(0x200000).unwrap().permissions, Permissions::READ_EXECUTE);
    assert_eq!(memory.read_u64(0x200000).unwrap(), 0x1122334455667788);

    // Ro check
    assert_eq!(memory.query(0x201000).unwrap().permissions, Permissions::READ);
    assert_eq!(memory.read_u64(0x201000).unwrap(), 0xaabbccddeeff0011);

    // Data check
    assert_eq!(memory.query(0x202000).unwrap().permissions, Permissions::READ_WRITE);
    assert_eq!(memory.read_u64(0x202000).unwrap(), 0x9988776655443322);

    // BSS check (zeroed)
    assert_eq!(memory.read_u64(0x203000).unwrap(), 0);
}

#[test]
fn invalid_magic_or_version_is_rejected() {
    let mut bad_magic = create_sample_nso(0);
    bad_magic[0] = b'M';
    assert!(NsoImage::parse(&bad_magic).is_err());

    let mut bad_version = create_sample_nso(0);
    bad_version[4] = 1;
    assert!(NsoImage::parse(&bad_version).is_err());
}

#[test]
fn nso_program_loads_multiple_modules_sequentially() {
    let nso1_bytes = create_sample_nso(0x1000);
    let nso2_bytes = create_sample_nso(0x2000);
    let nso1 = NsoImage::parse(&nso1_bytes).unwrap();
    let nso2 = NsoImage::parse(&nso2_bytes).unwrap();

    let mut memory = AddressSpace::default();
    let mut program = emulator_loader::NsoProgram::new();

    let mod1 = program.load_module("main", &nso1, &mut memory, 0x100000).unwrap();
    assert_eq!(mod1.base, 0x100000);
    assert_eq!(mod1.end, 0x104000);

    // Second module should automatically place after first module if next_base is smaller
    let mod2 = program.load_module("subsdk0", &nso2, &mut memory, 0x100000).unwrap();
    assert_eq!(mod2.base, 0x104000);
    assert_eq!(mod2.end, 0x109000); // 3 pages code/data + 2 pages BSS = 5 pages = 0x5000

    assert_eq!(program.modules().len(), 2);
    assert_eq!(program.modules()[0].name, "main");
    assert_eq!(program.modules()[1].name, "subsdk0");

    // Duplicate module names are rejected
    assert!(program.load_module("main", &nso1, &mut memory, 0x200000).is_err());
}

