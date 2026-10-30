use emulator_memory::{Access, AddressSpace, Mapping, MemoryError, Permissions};

#[test]
fn aliases_and_protection_fragments_keep_shared_identity_and_visible_writes() {
    let mut cpu = AddressSpace::with_limit(64);
    cpu.map(0x1000, vec![0; 16], Permissions::READ_WRITE)
        .unwrap();
    let view = cpu.pin(0x1000, 16, Permissions::READ_WRITE).unwrap();
    let id = view.backing_ranges().next().unwrap().id;
    cpu.map_alias(0x2000, &view, Permissions::READ_WRITE)
        .unwrap();
    assert_eq!(cpu.mapped_bytes(), 32);
    assert_eq!(cpu.resident_bytes(), 16);
    cpu.write_u64(0x2004, 42).unwrap();
    assert_eq!(cpu.read_u64(0x1004).unwrap(), 42);
    cpu.protect(0x1004, 8, Permissions::READ).unwrap();
    assert!(cpu.write_u64(0x1004, 1).is_err());
    cpu.write_u64(0x2004, 99).unwrap();
    assert_eq!(cpu.read_u64(0x1004).unwrap(), 99);
    assert!(cpu.pin(0x1000, 16, Permissions::READ_WRITE).is_err());
    let protected = cpu.pin(0x1000, 16, Permissions::READ).unwrap();
    assert!(protected.backing_ranges().all(|r| r.id == id));
    assert_eq!(cpu.resident_bytes(), 16);
    cpu.unmap(0x1000, 16).unwrap();
    view.write(4, &7_u64.to_le_bytes()).unwrap();
    assert_eq!(cpu.read_u64(0x2004).unwrap(), 7);
}
#[test]
fn retained_views_remain_charged_and_address_reuse_cannot_redirect_them() {
    let mut cpu = AddressSpace::with_limit(8);
    cpu.map(0x1000, vec![1; 8], Permissions::READ_WRITE)
        .unwrap();
    let view = cpu.pin(0x1000, 8, Permissions::READ).unwrap();
    let old = view.backing_ranges().next().unwrap().id;
    cpu.unmap(0x1000, 8).unwrap();
    assert_eq!(cpu.mapped_bytes(), 0);
    assert_eq!(cpu.resident_bytes(), 8);
    assert_eq!(
        cpu.map(0x1000, vec![2; 8], Permissions::READ),
        Err(MemoryError::AllocationLimit)
    );
    let mut data = [0; 8];
    view.read(0, &mut data).unwrap();
    assert_eq!(data, [1; 8]);
    drop(view);
    assert_eq!(cpu.resident_bytes(), 0);
    cpu.map(0x1000, vec![2; 8], Permissions::READ).unwrap();
    assert_ne!(
        cpu.pin(0x1000, 8, Permissions::READ)
            .unwrap()
            .backing_ranges()
            .next()
            .unwrap()
            .id,
        old
    );
}
#[test]
fn views_cross_backings_and_validate_all_permissions_and_bounds_before_writes() {
    let mut cpu = AddressSpace::default();
    cpu.map(0x1000, vec![1; 4], Permissions::READ_WRITE)
        .unwrap();
    cpu.map(0x1004, vec![2; 4], Permissions::READ_WRITE)
        .unwrap();
    let view = cpu.pin(0x1002, 4, Permissions::READ_WRITE).unwrap();
    assert_eq!(view.backing_ranges().count(), 2);
    view.write(0, &[3, 4, 5, 6]).unwrap();
    assert_eq!(cpu.read_u32(0x1002).unwrap(), 0x06050403);
    assert!(view.write(2, &[9; 4]).is_err());
    assert_eq!(cpu.read_u32(0x1002).unwrap(), 0x06050403);
    let slice = view.slice(1, 2).unwrap();
    let mut data = [0; 2];
    slice.read(0, &mut data).unwrap();
    assert_eq!(data, [4, 5]);
    assert!(view.slice(usize::MAX, 2).is_err());
    cpu.protect(0x1004, 4, Permissions::READ).unwrap();
    assert!(cpu.pin(0x1000, 8, Permissions::READ_WRITE).is_err());
    let read = cpu.pin(0x1000, 8, Permissions::READ).unwrap();
    assert!(read.write(0, &[0; 8]).is_err());
    assert!(cpu
        .map_alias(0x2000, &read, Permissions::READ_WRITE)
        .is_err());
    assert!(cpu.query(0x2000).is_none());
}
#[test]
fn failed_backing_allocation_and_alias_publication_restore_accounting() {
    let mut cpu = AddressSpace::with_limit(16);
    assert!(cpu
        .map_batch(vec![
            Mapping {
                start: 0,
                bytes: vec![1; 8],
                permissions: Permissions::READ
            },
            Mapping {
                start: 4,
                bytes: vec![2; 8],
                permissions: Permissions::READ
            }
        ])
        .is_err());
    assert_eq!(cpu.resident_bytes(), 0);
    assert_eq!(cpu.mapped_bytes(), 0);
    cpu.reserve(0x1000, 16).unwrap();
    assert!(cpu.pin(0x1000, 4, Permissions::READ).is_err());
    cpu.commit(0x1000, vec![0; 8], Permissions::READ).unwrap();
    let view = cpu.pin(0x1000, 8, Permissions::READ).unwrap();
    assert!(cpu.map_alias(0x1004, &view, Permissions::READ).is_err());
    assert_eq!(cpu.resident_bytes(), 8);
    assert_eq!(cpu.mapped_bytes(), 8);
    assert!(matches!(
        cpu.pin(0x1000, 8, Permissions::EXECUTE),
        Err(MemoryError::PermissionDenied {
            access: Access::Execute,
            ..
        })
    ));
}
