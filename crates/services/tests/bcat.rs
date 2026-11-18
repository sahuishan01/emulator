//! `IDeliveryCacheStorageService` enumeration over a real B buffer.
mod common;
use common::Rig;
use emulator_memory::Permissions;
use emulator_services::bcat::DeliveryCacheStorage;

#[test]
fn empty_delivery_cache_enumerates_no_directories() {
    let mut rig = Rig::new("bcatcache", Box::new(DeliveryCacheStorage));
    rig.m.map(0x30000, vec![0xee; 0x1000], Permissions::READ_WRITE).unwrap();
    rig.call(10, &[], &[(0x30000, 0x20 * 4)], None, &[]);
    assert_eq!((rig.result(), rig.out(4)), (0, 0i32.to_le_bytes().to_vec()));
    assert_eq!(rig.m.read_u8(0x30000).unwrap(), 0xee, "buffer untouched");
    // The output buffer is required.
    assert!(rig.try_call(10, &[], &[], None, &[]).is_err());
}
