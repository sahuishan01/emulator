use emulator_loader::reloc::{apply_relocations, DT_NULL, DT_RELA, DT_RELAENT, DT_RELASZ, R_AARCH64_RELATIVE};
use emulator_memory::{AddressSpace, Permissions};

#[test]
fn test_relative_relocations() {
    let mut mem = AddressSpace::default();
    let base = 0x7100_0000u64;

    // We map a 64KiB region for base
    mem.map(base, vec![0u8; 0x10000], Permissions::READ_WRITE).unwrap();

    // Setup .dynamic section at offset 0x1000
    // DT_RELA = 7, offset = 0x2000
    // DT_RELASZ = 8, size = 48 (2 entries of 24 bytes)
    // DT_RELAENT = 9, ent = 24
    // DT_NULL = 0, val = 0
    let dyn_offset = 0x1000u64;
    let rela_offset = 0x2000u64;

    mem.write_u64(base + dyn_offset, DT_RELA as u64).unwrap();
    mem.write_u64(base + dyn_offset + 8, rela_offset).unwrap();

    mem.write_u64(base + dyn_offset + 16, DT_RELASZ as u64).unwrap();
    mem.write_u64(base + dyn_offset + 24, 48).unwrap();

    mem.write_u64(base + dyn_offset + 32, DT_RELAENT as u64).unwrap();
    mem.write_u64(base + dyn_offset + 40, 24).unwrap();

    mem.write_u64(base + dyn_offset + 48, DT_NULL as u64).unwrap();
    mem.write_u64(base + dyn_offset + 56, 0).unwrap();

    // Entry 0: patch at offset 0x3000, addend 0x500
    mem.write_u64(base + rela_offset, 0x3000).unwrap();
    mem.write_u64(base + rela_offset + 8, R_AARCH64_RELATIVE as u64).unwrap();
    mem.write_u64(base + rela_offset + 16, 0x500).unwrap();

    // Entry 1: patch at offset 0x3008, addend -0x100
    mem.write_u64(base + rela_offset + 24, 0x3008).unwrap();
    mem.write_u64(base + rela_offset + 32, R_AARCH64_RELATIVE as u64).unwrap();
    mem.write_u64(base + rela_offset + 40, (-0x100i64) as u64).unwrap();

    let count = apply_relocations(&mut mem, base, dyn_offset, 0x1000).unwrap();
    assert_eq!(count, 2);

    assert_eq!(mem.read_u64(base + 0x3000).unwrap(), base + 0x500);
    assert_eq!(mem.read_u64(base + 0x3008).unwrap(), base - 0x100);
}
