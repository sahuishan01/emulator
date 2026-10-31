use emulator_a64::Cpu;
use emulator_kernel::{
    HandleError, Kernel, KernelError, MAX_TRANSFER_MEMORY_OBJECTS, SVC_CLOSE_HANDLE,
    SVC_CREATE_TRANSFER_MEMORY, SVC_EXIT_PROCESS,
};
use emulator_memory::{AddressSpace, Permissions};
fn create(
    k: &mut Kernel,
    c: &mut Cpu,
    m: &mut AddressSpace,
    address: u64,
    size: u64,
    permission: u64,
) -> Result<u32, KernelError> {
    c.x[1] = address;
    c.x[2] = size;
    c.x[3] = permission;
    k.dispatch(SVC_CREATE_TRANSFER_MEMORY, c, m)?;
    Ok(c.x[1] as u32)
}
#[test]
fn transfer_svc_lends_real_bytes_until_handle_close_and_process_exit() {
    let mut k = Kernel::default();
    let mut c = Cpu::new(0, 0);
    let mut m = AddressSpace::default();
    m.map(0x4000, vec![7; 4096], Permissions::READ_WRITE)
        .unwrap();
    // The SVC consumes W3; high X3 bits must not change the permission.
    let handle = create(&mut k, &mut c, &mut m, 0x4000, 4096, 0x1234_5678_0000_0000).unwrap();
    assert_eq!(c.x[0], 0);
    assert!(m.read_u64(0x4000).is_err());
    assert_eq!(k.live_transfer_memory_count(), 1);
    c.x[0] = u64::from(handle);
    k.dispatch(SVC_CLOSE_HANDLE, &mut c, &mut m).unwrap();
    assert_eq!(k.live_transfer_memory_count(), 0);
    assert_eq!(m.read_u64(0x4000).unwrap(), u64::from_le_bytes([7; 8]));
    let next = create(&mut k, &mut c, &mut m, 0x4000, 4096, 1).unwrap();
    assert!(next > handle);
    assert!(m.write_u64(0x4000, 1).is_err());
    k.dispatch(SVC_EXIT_PROCESS, &mut c, &mut m).unwrap();
    assert_eq!(k.live_transfer_memory_count(), 0);
    m.write_u64(0x4000, 42).unwrap();
    assert_eq!(m.lent_bytes(), 0);
}
#[test]
fn invalid_transfer_arguments_and_read_only_memory_preserve_cpu_and_memory() {
    for (address, size, permission) in [
        (0x4001, 4096, 0),
        (0x4000, 0, 0),
        (0x4000, 4097, 0),
        (0x4000, 4096, 2),
        (0x4000, 4096, 4),
        (u64::MAX - 4095, 8192, 0),
        (0x4000, 17 * 1024 * 1024, 0),
        (emulator_kernel::THREAD_TLS_ARENA, 4096, 0),
    ] {
        let mut k = Kernel::default();
        let mut c = Cpu::new(0, 0);
        let mut m = AddressSpace::default();
        m.map(0x4000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        c.x[1] = address;
        c.x[2] = size;
        c.x[3] = permission;
        let saved = c.clone();
        assert!(k
            .dispatch(SVC_CREATE_TRANSFER_MEMORY, &mut c, &mut m)
            .is_err());
        assert_eq!(c, saved);
        assert_eq!(m.lent_bytes(), 0);
        assert_eq!(k.live_handle_count(), 1);
    }
    let mut k = Kernel::default();
    let mut c = Cpu::new(0, 0);
    let mut m = AddressSpace::default();
    m.map(0x4000, vec![0; 4096], Permissions::READ).unwrap();
    assert!(create(&mut k, &mut c, &mut m, 0x4000, 4096, 0).is_err());
    assert_eq!(m.lent_bytes(), 0);
}
#[test]
fn object_quota_counts_real_objects_and_recovers_after_close() {
    let mut k = Kernel::default();
    let mut c = Cpu::new(0, 0);
    let mut m = AddressSpace::default();
    let mut first = 0;
    for i in 0..=MAX_TRANSFER_MEMORY_OBJECTS {
        m.map(
            0x10000 + i as u64 * 4096,
            vec![0; 4096],
            Permissions::READ_WRITE,
        )
        .unwrap();
    }
    for i in 0..MAX_TRANSFER_MEMORY_OBJECTS {
        let h = create(&mut k, &mut c, &mut m, 0x10000 + i as u64 * 4096, 4096, 0).unwrap();
        if i == 0 {
            first = h;
        }
    }
    assert!(matches!(
        create(
            &mut k,
            &mut c,
            &mut m,
            0x10000 + MAX_TRANSFER_MEMORY_OBJECTS as u64 * 4096,
            4096,
            0
        ),
        Err(KernelError::TransferMemoryLimit)
    ));
    c.x[0] = first as u64;
    k.dispatch(SVC_CLOSE_HANDLE, &mut c, &mut m).unwrap();
    create(
        &mut k,
        &mut c,
        &mut m,
        0x10000 + MAX_TRANSFER_MEMORY_OBJECTS as u64 * 4096,
        4096,
        0,
    )
    .unwrap();
    k.dispatch(SVC_EXIT_PROCESS, &mut c, &mut m).unwrap();
    assert_eq!(k.live_transfer_memory_count(), 0);
    assert_eq!(m.lent_bytes(), 0);
}
#[derive(Clone, Debug)]
struct Noop;
impl emulator_kernel::service::Service for Noop {
    fn clone_for_request(&self) -> Box<dyn emulator_kernel::service::Service> {
        Box::new(self.clone())
    }
    fn dispatch(
        &mut self,
        _: &emulator_ipc::Request,
    ) -> Result<emulator_kernel::service::ServiceReply, emulator_kernel::service::ServiceError>
    {
        Ok(Default::default())
    }
}
#[test]
fn a_full_handle_table_cannot_publish_a_memory_loan() {
    let mut k = Kernel::default();
    let mut c = Cpu::new(0, 0);
    let mut m = AddressSpace::default();
    m.map(0x1000, b"noop\0".to_vec(), Permissions::READ)
        .unwrap();
    m.map(0x4000, vec![0; 4096], Permissions::READ_WRITE)
        .unwrap();
    k.register_named_port("noop", Box::new(Noop)).unwrap();
    for _ in 0..255 {
        c.x[1] = 0x1000;
        k.dispatch(emulator_kernel::SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m)
            .unwrap();
    }
    assert_eq!(
        create(&mut k, &mut c, &mut m, 0x4000, 4096, 0),
        Err(KernelError::Handle(HandleError::Limit))
    );
    assert_eq!(m.lent_bytes(), 0);
    assert_eq!(k.live_transfer_memory_count(), 0);
    m.write_u64(0x4000, 42).unwrap();
}
