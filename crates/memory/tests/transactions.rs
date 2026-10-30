use emulator_memory::{Access, AddressSpace, Mapping, MemoryError, Permissions};

#[test]
fn batch_overlap_rolls_back_every_new_mapping() {
    let mut memory = AddressSpace::default();
    memory.map(0x3000, vec![7; 16], Permissions::READ).unwrap();
    let result = memory.map_batch(vec![
        Mapping {
            start: 0x1000,
            bytes: vec![1; 16],
            permissions: Permissions::READ,
        },
        Mapping {
            start: 0x3008,
            bytes: vec![2; 16],
            permissions: Permissions::READ,
        },
    ]);
    assert_eq!(result, Err(MemoryError::OverlappingMapping));
    assert!(memory.query(0x1000).is_none());
    assert_eq!(memory.read_u32(0x3000).unwrap(), 0x07070707);
}

#[test]
fn batch_new_mappings_cannot_overlap_each_other() {
    let mut memory = AddressSpace::default();
    assert_eq!(
        memory.map_batch(vec![
            Mapping {
                start: 0x1000,
                bytes: vec![0; 16],
                permissions: Permissions::READ
            },
            Mapping {
                start: 0x1008,
                bytes: vec![0; 16],
                permissions: Permissions::READ
            },
        ]),
        Err(MemoryError::OverlappingMapping)
    );
    assert_eq!(memory.mapped_bytes(), 0);
}

#[test]
fn cross_region_reads_and_writes_are_validated_before_mutation() {
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, vec![1; 4], Permissions::READ_WRITE)
        .unwrap();
    memory
        .map(0x1004, vec![2; 4], Permissions::READ_WRITE)
        .unwrap();
    memory.write_u64(0x1000, 0x0123456789abcdef).unwrap();
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x0123456789abcdef);
    memory.protect(0x1004, 4, Permissions::READ).unwrap();
    assert!(matches!(
        memory.write_u64(0x1000, 0),
        Err(MemoryError::PermissionDenied {
            access: Access::Write,
            ..
        })
    ));
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x0123456789abcdef);
}

#[test]
fn reserved_ranges_commit_and_split_without_becoming_accessible_early() {
    let mut memory = AddressSpace::with_limit(8);
    memory.reserve(0x1000, 0x10000).unwrap();
    assert_eq!(memory.mapped_bytes(), 0);
    assert!(!memory.query(0x1000).unwrap().committed);
    assert!(memory.read_u32(0x1000).is_err());
    memory
        .commit(0x1004, vec![42; 8], Permissions::READ_WRITE)
        .unwrap();
    assert_eq!(memory.mapped_bytes(), 8);
    assert!(memory.query(0x1004).unwrap().committed);
    assert!(!memory.query(0x1003).unwrap().committed);
    assert!(!memory.query(0x100c).unwrap().committed);
    assert_eq!(
        memory.commit(0x1004, vec![0; 4], Permissions::READ),
        Err(MemoryError::NotReserved { address: 0x1004 })
    );
    assert_eq!(
        memory.commit(0x100c, vec![0], Permissions::READ),
        Err(MemoryError::AllocationLimit)
    );
    assert!(!memory.query(0x100c).unwrap().committed);
}

#[test]
fn protect_and_unmap_preserve_fragments_and_fail_transactionally_on_holes() {
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, (0..16).collect(), Permissions::READ_WRITE)
        .unwrap();
    memory.protect(0x1004, 8, Permissions::READ).unwrap();
    assert!(memory.write_u32(0x1004, 0).is_err());
    memory.write_u32(0x100c, 42).unwrap();
    memory.unmap(0x1006, 4).unwrap();
    assert!(memory.query(0x1006).is_none());
    assert!(memory.read_u32(0x1004).is_err());
    assert!(memory.protect(0x1000, 16, Permissions::NONE).is_err());
    assert_eq!(memory.read_u32(0x100c).unwrap(), 42);
    assert_eq!(memory.read_u32(0x1000).unwrap(), 0x03020100);
}

#[test]
fn allocation_cap_failure_does_not_publish_any_region() {
    let mut memory = AddressSpace::with_limit(4);
    assert_eq!(
        memory.map_batch(vec![
            Mapping {
                start: 0,
                bytes: vec![0; 4],
                permissions: Permissions::READ
            },
            Mapping {
                start: 4,
                bytes: vec![0],
                permissions: Permissions::READ
            },
        ]),
        Err(MemoryError::AllocationLimit)
    );
    assert!(memory.query(0).is_none());
}
