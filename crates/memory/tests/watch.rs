use emulator_memory::{AddressSpace, Permissions, WatchHit};

#[test]
fn write_watch_logs_only_overlapping_writes_and_drains() {
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, vec![0; 0x100], Permissions::READ_WRITE)
        .unwrap();
    memory.set_write_watch(Some((0x1010, 8)));
    memory.write_u64(0x1000, 1).unwrap(); // ends at 0x1008: outside
    memory.write_u32(0x100e, 0xaabbccdd).unwrap(); // straddles start
    memory.write_u8(0x1018, 9).unwrap(); // first byte past end: outside
    assert_eq!(
        memory.take_watch_hits(),
        vec![WatchHit {
            address: 0x100e,
            bytes: vec![0xdd, 0xcc, 0xbb, 0xaa]
        }]
    );
    assert!(memory.take_watch_hits().is_empty());
    memory.set_write_watch(None);
    memory.write_u64(0x1010, 2).unwrap();
    assert!(memory.take_watch_hits().is_empty());
}

#[test]
fn rejected_write_is_still_logged_but_memory_unchanged() {
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, vec![0; 0x10], Permissions::READ)
        .unwrap();
    memory.set_write_watch(Some((0x1000, 0x10)));
    assert!(memory.write_u8(0x1000, 1).is_err());
    assert_eq!(memory.take_watch_hits().len(), 1);
    assert_eq!(memory.read_u8(0x1000).unwrap(), 0);
}
