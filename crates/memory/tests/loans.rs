use emulator_memory::{AddressSpace, Mapping, MemoryError, Permissions};
#[test]
fn partial_lending_preserves_backing_and_restores_after_the_last_lease() {
    let mut memory = AddressSpace::with_limit(16384);
    memory
        .map(0x1000, vec![7; 12288], Permissions::READ_WRITE)
        .unwrap();
    let original = memory
        .pin(0x2000, 4096, Permissions::READ)
        .unwrap()
        .backing_ranges()
        .next()
        .unwrap()
        .id;
    let lease = memory.lend(0x2000, 4096, Permissions::NONE).unwrap();
    let clone = lease.clone();
    assert_eq!(memory.lent_bytes(), 4096);
    assert!(memory.query(0x2000).unwrap().borrowed);
    let mut bytes = [0xaa; 8];
    assert!(memory.read(0x2000, &mut bytes).is_err());
    assert_eq!(bytes, [0xaa; 8]);
    memory.write_u64(0x1000, 9).unwrap();
    memory.protect(0x3000, 4096, Permissions::READ).unwrap();
    lease.write(0, &42_u64.to_le_bytes()).unwrap();
    drop(lease);
    assert!(memory.read_u64(0x2000).is_err());
    drop(clone);
    assert_eq!(memory.lent_bytes(), 0);
    assert_eq!(memory.read_u64(0x2000).unwrap(), 42);
    assert!(!memory.query(0x2000).unwrap().borrowed);
    assert_eq!(
        memory.query(0x2000).unwrap().permissions,
        Permissions::READ_WRITE
    );
    assert_eq!(
        memory
            .pin(0x2000, 4096, Permissions::READ)
            .unwrap()
            .backing_ranges()
            .next()
            .unwrap()
            .id,
        original
    );
    assert_eq!(memory.resident_bytes(), 12288);
}
#[test]
fn allowed_owner_rights_do_not_allow_mapping_mutation_during_a_loan() {
    for permission in [Permissions::READ, Permissions::READ_WRITE] {
        let mut m = AddressSpace::with_limit(8192);
        m.map(0x1000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        let lease = m.lend(0x1000, 4096, permission).unwrap();
        assert_eq!(m.read_u64(0x1000).unwrap(), 0);
        if permission == Permissions::READ {
            assert!(m.write_u64(0x1000, 1).is_err());
        } else {
            m.write_u64(0x1000, 1).unwrap();
        }
        assert_eq!(
            m.protect(0x1000, 4096, Permissions::READ_WRITE),
            Err(MemoryError::LoanConflict)
        );
        assert_eq!(m.unmap(0x1000, 4096), Err(MemoryError::LoanConflict));
        let view = m.pin(0x1000, 4096, Permissions::READ).unwrap();
        assert_eq!(
            m.map_alias(0x3000, &view, Permissions::READ),
            Err(MemoryError::LoanConflict)
        );
        drop(view);
        assert!(m.lend(0x1000, 4096, Permissions::NONE).is_err());
        drop(lease);
        m.unmap(0x1000, 4096).unwrap();
        assert_eq!(m.resident_bytes(), 0);
    }
}
#[test]
fn outstanding_views_aliases_bad_rights_and_gaps_fail_without_lending() {
    let mut m = AddressSpace::with_limit(16384);
    m.map(0x1000, vec![1; 4096], Permissions::READ_WRITE)
        .unwrap();
    let view = m.pin(0x1000, 4096, Permissions::READ_WRITE).unwrap();
    assert!(matches!(
        m.lend(0x1000, 4096, Permissions::NONE),
        Err(MemoryError::LoanConflict)
    ));
    m.map_alias(0x3000, &view, Permissions::READ_WRITE).unwrap();
    drop(view);
    assert!(matches!(
        m.lend(0x1000, 4096, Permissions::NONE),
        Err(MemoryError::LoanConflict)
    ));
    m.unmap(0x3000, 4096).unwrap();
    for (address, size, rights) in [
        (0x1000, 4096, Permissions::EXECUTE),
        (0x1000, 4096, Permissions::WRITE),
        (0x1000, 8192, Permissions::NONE),
        (u64::MAX, 8, Permissions::NONE),
    ] {
        assert!(m.lend(address, size, rights).is_err());
        assert_eq!(m.lent_bytes(), 0);
        assert_eq!(
            m.query(0x1000).unwrap().permissions,
            Permissions::READ_WRITE
        );
    }
    let lease = m.lend(0x1000, 4096, Permissions::NONE).unwrap();
    drop(lease);
    m.write_u64(0x1000, 42).unwrap();
    let mut alias = AddressSpace::with_limit(4096);
    alias
        .map(0x1000, vec![0; 2048], Permissions::READ_WRITE)
        .unwrap();
    let view = alias.pin(0x1000, 2048, Permissions::READ_WRITE).unwrap();
    alias
        .map_alias(0x1800, &view, Permissions::READ_WRITE)
        .unwrap();
    drop(view);
    assert!(matches!(
        alias.lend(0x1000, 4096, Permissions::NONE),
        Err(MemoryError::LoanConflict)
    ));
    assert_eq!(alias.lent_bytes(), 0);
}
#[test]
fn failed_region_splitting_keeps_all_rights_and_drops_temporary_pins() {
    let mut m = AddressSpace::with_limit(20000);
    let mut maps = vec![Mapping {
        start: 0x1000,
        bytes: vec![0; 12288],
        permissions: Permissions::READ_WRITE,
    }];
    maps.extend((0..4095).map(|i| Mapping {
        start: 0x10000 + i,
        bytes: vec![0],
        permissions: Permissions::READ_WRITE,
    }));
    m.map_batch(maps).unwrap();
    assert!(matches!(
        m.lend(0x2000, 4096, Permissions::NONE),
        Err(MemoryError::TooManyMappings)
    ));
    assert_eq!(m.lent_bytes(), 0);
    assert_eq!(m.query(0x2000).unwrap().start, 0x1000);
    m.write_u64(0x2000, 42).unwrap();
    let lease = m.lend(0x1000, 12288, Permissions::NONE).unwrap();
    drop(lease);
    assert_eq!(m.read_u64(0x2000).unwrap(), 42);
}
