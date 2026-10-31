use emulator_a64::Cpu;
use emulator_ipc::Request;
use emulator_kernel::{
    service::{Service, ServiceContext, ServiceError, ServiceReply},
    Kernel, KernelError, CURRENT_PROCESS_HANDLE, SVC_CLOSE_HANDLE, SVC_CONNECT_TO_NAMED_PORT,
    SVC_CREATE_TRANSFER_MEMORY, SVC_SEND_SYNC_REQUEST,
};
use emulator_memory::{AddressSpace, Permissions};
use emulator_services::NvDriver;
struct Rig {
    k: Kernel,
    c: Cpu,
    m: AddressSpace,
    tmem: u32,
    session: u32,
}
impl Rig {
    fn new(permission: u64, service: Box<dyn Service>) -> Self {
        let mut k = Kernel::default();
        let mut c = Cpu::new(0, 0);
        let mut m = AddressSpace::default();
        m.map(0x1000, b"nv:\0".to_vec(), Permissions::READ).unwrap();
        m.map(0x2000, vec![0; 256], Permissions::READ_WRITE)
            .unwrap();
        m.map(0x4000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        m.write_u64(0x4000, 42).unwrap();
        c.x[1] = 0x4000;
        c.x[2] = 4096;
        c.x[3] = permission;
        k.dispatch(SVC_CREATE_TRANSFER_MEMORY, &mut c, &mut m)
            .unwrap();
        let tmem = c.x[1] as u32;
        k.register_named_port("nv:", service).unwrap();
        c.tpidrro_el0 = 0x2000;
        c.x[1] = 0x1000;
        k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m)
            .unwrap();
        let session = c.x[1] as u32;
        Self {
            k,
            c,
            m,
            tmem,
            session,
        }
    }
    fn request(
        &mut self,
        command: u32,
        size: u32,
        process: u32,
        tmem: u32,
    ) -> Result<(), KernelError> {
        let mut b = [0; 256];
        b[..4].copy_from_slice(&4_u32.to_le_bytes());
        b[4..8].copy_from_slice(&0x80000009_u32.to_le_bytes());
        b[8..12].copy_from_slice(&4_u32.to_le_bytes());
        b[12..16].copy_from_slice(&process.to_le_bytes());
        b[16..20].copy_from_slice(&tmem.to_le_bytes());
        b[32..36].copy_from_slice(b"SFCI");
        b[40..44].copy_from_slice(&command.to_le_bytes());
        b[48..52].copy_from_slice(&size.to_le_bytes());
        self.call(&b)
    }
    fn call(&mut self, b: &[u8]) -> Result<(), KernelError> {
        self.m.write(0x2000, b).unwrap();
        self.c.x[0] = self.session as u64;
        self.k
            .dispatch(SVC_SEND_SYNC_REQUEST, &mut self.c, &mut self.m)
            .map(|_| ())
    }
    fn close(&mut self, handle: u32) {
        self.c.x[0] = handle as u64;
        self.k
            .dispatch(SVC_CLOSE_HANDLE, &mut self.c, &mut self.m)
            .unwrap();
        assert_eq!(self.c.x[0], 0);
    }
    fn initialize(&mut self) -> Result<(), KernelError> {
        self.request(3, 4096, CURRENT_PROCESS_HANDLE, self.tmem)
    }
}
#[test]
fn initialized_nv_client_retains_copied_memory_through_guest_close_and_session_clone() {
    let mut r = Rig::new(0, Box::new(NvDriver::default()));
    r.initialize().unwrap();
    assert_eq!(r.k.ipc_records()[0].copy_handles, 2);
    assert_eq!(r.m.read_u32(0x2018).unwrap(), 0);
    r.close(r.tmem);
    assert_eq!(r.k.live_transfer_memory_count(), 1);
    assert!(r.m.read_u64(0x4000).is_err());
    let mut b = [0; 256];
    b[..4].copy_from_slice(&5_u32.to_le_bytes());
    b[4..8].copy_from_slice(&8_u32.to_le_bytes());
    b[16..20].copy_from_slice(b"SFCI");
    b[24..28].copy_from_slice(&2_u32.to_le_bytes());
    r.call(&b).unwrap();
    let clone = r.m.read_u32(0x200c).unwrap();
    r.close(r.session);
    assert!(r.m.read_u64(0x4000).is_err());
    r.close(clone);
    assert_eq!(r.k.live_transfer_memory_count(), 0);
    assert_eq!(r.m.lent_bytes(), 0);
    assert_eq!(r.m.read_u64(0x4000).unwrap(), 42);
}
#[test]
fn invalid_copy_ids_types_order_and_size_never_adopt_a_transfer_arena() {
    for case in 0..5 {
        let mut r = Rig::new(0, Box::new(NvDriver::default()));
        let result = match case {
            0 => r.request(3, 8192, CURRENT_PROCESS_HANDLE, r.tmem),
            1 => r.request(3, 4096, 0xdead, r.tmem),
            2 => r.request(3, 4096, r.tmem, CURRENT_PROCESS_HANDLE),
            3 => r.request(3, 4096, r.session, r.tmem),
            _ => r.request(3, 4096, CURRENT_PROCESS_HANDLE, 0xffff8000),
        };
        assert!(result.is_err());
        assert!(r.k.ipc_records().is_empty());
        r.close(r.tmem);
        assert_eq!(r.k.live_transfer_memory_count(), 0);
        assert_eq!(r.m.read_u64(0x4000).unwrap(), 42);
        if case == 1 {
            assert!(matches!(result, Err(KernelError::Handle(_))));
        }
        if case == 3 {
            assert!(matches!(result, Err(KernelError::UnsupportedCopyHandle(_))));
        }
    }
}
#[test]
fn unsupported_permissions_duplicate_initialization_and_device_calls_fail_explicitly() {
    let mut r = Rig::new(1, Box::new(NvDriver::default()));
    assert_eq!(
        r.initialize(),
        Err(KernelError::Service(ServiceError::InvalidTransferMemory))
    );
    r.close(r.tmem);
    assert_eq!(
        r.m.query(0x4000).unwrap().permissions,
        Permissions::READ_WRITE
    );
    let mut r = Rig::new(0, Box::new(NvDriver::default()));
    r.initialize().unwrap();
    assert_eq!(
        r.initialize(),
        Err(KernelError::Service(ServiceError::AlreadyInitialized))
    );
    assert!(r.request(4, 4096, CURRENT_PROCESS_HANDLE, r.tmem).is_err());
}
#[derive(Clone, Debug, Default)]
struct Oversized {
    nv: NvDriver,
}
impl Service for Oversized {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, r: &Request) -> Result<ServiceReply, ServiceError> {
        self.nv.dispatch(r)
    }
    fn prepare(
        &mut self,
        r: &Request,
        c: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        let mut request = r.clone();
        request.command_id = 3;
        let mut reply = self.nv.prepare(&request, c)?;
        if r.command_id == 99 {
            reply.data = vec![0; 300];
        }
        Ok(reply)
    }
}
#[test]
fn rejected_reply_releases_candidate_copied_objects_without_initializing_live_state() {
    let mut r = Rig::new(0, Box::new(Oversized::default()));
    assert!(matches!(
        r.request(99, 4096, CURRENT_PROCESS_HANDLE, r.tmem),
        Err(KernelError::Ipc(_))
    ));
    assert!(r.k.ipc_records().is_empty());
    r.initialize().unwrap();
    r.close(r.tmem);
    assert!(r.m.read_u64(0x4000).is_err());
    r.close(r.session);
    assert_eq!(r.k.live_transfer_memory_count(), 0);
    assert_eq!(r.m.read_u64(0x4000).unwrap(), 42);
}
#[test]
fn unwritable_reply_and_unhandled_copies_do_not_retain_service_references() {
    let mut r = Rig::new(0, Box::new(NvDriver::default()));
    let mut b = [0; 256];
    b[..4].copy_from_slice(&4_u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x80000009_u32.to_le_bytes());
    b[8..12].copy_from_slice(&4_u32.to_le_bytes());
    b[12..16].copy_from_slice(&CURRENT_PROCESS_HANDLE.to_le_bytes());
    b[16..20].copy_from_slice(&r.tmem.to_le_bytes());
    b[32..36].copy_from_slice(b"SFCI");
    b[40..44].copy_from_slice(&3_u32.to_le_bytes());
    b[48..52].copy_from_slice(&4096_u32.to_le_bytes());
    r.m.write(0x2000, &b).unwrap();
    r.m.protect(0x2000, 256, Permissions::READ).unwrap();
    r.c.x[0] = r.session as u64;
    assert!(r
        .k
        .dispatch(SVC_SEND_SYNC_REQUEST, &mut r.c, &mut r.m)
        .is_err());
    r.close(r.tmem);
    assert_eq!(r.k.live_transfer_memory_count(), 0);
    assert_eq!(r.m.read_u64(0x4000).unwrap(), 42);
}

#[test]
fn services_without_copy_handle_contracts_reject_them_before_mutation() {
    let mut r = Rig::new(0, Box::new(emulator_services::DiagnosticCounter::default()));
    assert_eq!(
        r.request(4, 4096, CURRENT_PROCESS_HANDLE, r.tmem),
        Err(KernelError::Service(ServiceError::InvalidCopyHandleCount(
            2
        )))
    );
    r.close(r.tmem);
    assert_eq!(r.k.live_transfer_memory_count(), 0);
    assert_eq!(r.m.read_u64(0x4000).unwrap(), 42);
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn message(
    command: u32,
    data: &[u32],
    input: Option<(u32, u32)>,
    output: Option<(u32, u32)>,
) -> [u8; 256] {
    let mut b = [0; 256];
    b[..4].copy_from_slice(
        &(4 | (u32::from(input.is_some()) << 20) | (u32::from(output.is_some()) << 24))
            .to_le_bytes(),
    );
    b[4..8].copy_from_slice(&(8 + data.len() as u32).to_le_bytes());
    let mut cursor = 8;
    for (address, size) in [input, output].into_iter().flatten() {
        b[cursor..cursor + 4].copy_from_slice(&size.to_le_bytes());
        b[cursor + 4..cursor + 8].copy_from_slice(&address.to_le_bytes());
        cursor += 12;
    }
    let cmif = (cursor + 15) & !15;
    b[cmif..cmif + 4].copy_from_slice(b"SFCI");
    b[cmif + 8..cmif + 12].copy_from_slice(&command.to_le_bytes());
    b[cmif + 16..cmif + 16 + data.len() * 4].copy_from_slice(&words(data));
    b
}
impl Rig {
    fn device_rig(service: Box<dyn Service>) -> Self {
        let mut r = Self::new(0, service);
        r.m.map(0x6000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        r.initialize().unwrap();
        r
    }
    fn open_device(&mut self, path: &[u8], command: u32) -> (u32, u32) {
        self.m.write(0x6000, path).unwrap();
        self.call(&message(
            command,
            &[],
            Some((0x6000, path.len() as u32)),
            None,
        ))
        .unwrap();
        (
            self.m.read_u32(0x2020).unwrap(),
            self.m.read_u32(0x2024).unwrap(),
        )
    }
    fn ioctl(&mut self, fd: u32, code: u32, args: &[u32], command: u32) -> u32 {
        self.m.write(0x6000, &words(args)).unwrap();
        let size = args.len() as u32 * 4;
        self.call(&message(
            command,
            &[fd, code],
            Some((0x6000, size)),
            Some((0x6000, size)),
        ))
        .unwrap();
        self.m.read_u32(0x2020).unwrap()
    }
    fn close_fd(&mut self, fd: u32) -> u32 {
        self.call(&message(2, &[fd], None, None)).unwrap();
        self.m.read_u32(0x2020).unwrap()
    }
}
#[test]
fn nvmap_client_shares_fds_through_clone_and_lends_until_last_receiver_close() {
    let mut r = Rig::device_rig(Box::new(NvDriver::default()));
    let (fd, error) = r.open_device(b"/dev/nvmap", 0);
    assert_eq!((fd, error), (1, 0));
    let mut clone = message(2, &[], None, None);
    clone[..4].copy_from_slice(&5_u32.to_le_bytes());
    r.call(&clone).unwrap();
    let cloned = r.m.read_u32(0x200c).unwrap();
    r.close(r.session);
    r.session = cloned;
    r.close(r.tmem);
    assert_eq!(r.ioctl(fd, 0xc0080101, &[8192, 0], 1), 0);
    let handle = r.m.read_u32(0x6004).unwrap();
    assert_eq!(handle, 1);
    assert_eq!(r.ioctl(fd, 0xc00c0109, &[handle, 1, 99], 1), 0);
    assert_eq!(r.m.read_u32(0x6008).unwrap(), 8192);
    assert_eq!(r.ioctl(fd, 0xc0180105, &[handle, 0, 99, 99, 99, 99], 1), 0);
    assert_eq!(r.m.read_u64(0x6008).unwrap(), 0);
    assert_eq!(r.m.read_u32(0x6010).unwrap(), 8192);
    assert_eq!(r.m.read_u32(0x6014).unwrap(), 0);
    assert_eq!(r.ioctl(fd, 0xc00c0109, &[handle, 1, 99], 1), 4);
    assert_eq!(r.m.read_u32(0x6008).unwrap(), 99);
    assert_eq!(r.close_fd(fd), 0);
    assert_eq!(r.close_fd(fd), 4);
    assert!(r.m.read_u64(0x4000).is_err());
    r.close(r.session);
    assert_eq!(r.m.read_u64(0x4000).unwrap(), 42);
    assert_eq!(r.k.live_transfer_memory_count(), 0);
}
#[test]
fn nvmap_rejects_unknown_devices_ioctls_handles_sizes_and_malformed_buffers() {
    let mut r = Rig::new(0, Box::new(NvDriver::default()));
    r.m.map(0x6000, vec![0; 4096], Permissions::READ_WRITE)
        .unwrap();
    assert_eq!(r.open_device(b"/dev/nvmap", 0), (0, 3));
    r.initialize().unwrap();
    for path in [b"/dev/nvhost-gpu".as_slice(), b"/dev/nvmap\0", b"\xff"] {
        let result = r.open_device(path, 0);
        assert_eq!(result.0, 0);
        assert_eq!(result.1, if path == b"/dev/nvhost-gpu" { 2 } else { 4 });
    }
    assert!(r.call(&message(0, &[], Some((0x6000, 0)), None)).is_err());
    let fd = r.open_device(b"/dev/nvmap", 0).0;
    for size in [0, 1, 4097, 64 * 1024 * 1024 + 4096] {
        assert_eq!(r.ioctl(fd, 0xc0080101, &[size, 77], 1), 4);
        assert_eq!(r.m.read_u32(0x6004).unwrap(), 77);
    }
    assert_eq!(r.ioctl(999, 0xc0080101, &[4096, 77], 1), 4);
    assert_eq!(r.ioctl(fd, 0xc0200104, &[1, 2, 3, 4, 5, 6, 7, 8], 1), 2);
    assert_eq!(r.m.read_u32(0x601c).unwrap(), 8);
    assert_eq!(r.ioctl(fd, 0xc0080101, &[4096, 77], 1), 0);
    let handle = r.m.read_u32(0x6004).unwrap();
    assert_eq!(handle, 1);
    assert_eq!(r.ioctl(fd, 0xc00c0109, &[handle, 2, 77], 1), 2);
    assert_eq!(r.m.read_u32(0x6008).unwrap(), 77);
    assert_eq!(r.ioctl(fd, 0xc0180105, &[handle, 1, 0, 0, 0, 0], 1), 4);
    assert_eq!(r.ioctl(fd, 0xc00c0109, &[handle, 1, 77], 1), 0);
    assert_eq!(r.m.read_u32(0x6008).unwrap(), 4096);
    // A readable input cannot grant an output write, nor can B alias the reply.
    for output in [(0x2000, 8), (0x7000, 8)] {
        assert!(r
            .call(&message(
                1,
                &[fd, 0xc0080101],
                Some((0x6000, 8)),
                Some(output)
            ))
            .is_err());
    }
    r.m.protect(0x6000, 4096, Permissions::READ).unwrap();
    assert!(r
        .call(&message(
            1,
            &[fd, 0xc0080101],
            Some((0x6000, 8)),
            Some((0x6000, 8))
        ))
        .is_err());
    r.m.protect(0x6000, 4096, Permissions::READ_WRITE).unwrap();
    assert_eq!(r.ioctl(fd, 0xc0080101, &[4096], 1), 4);
    assert_eq!(r.ioctl(fd, 0xc0080101, &[4096, 0], 1), 0);
    assert_eq!(r.m.read_u32(0x6004).unwrap(), 2);
}
#[derive(Clone, Debug, Default)]
struct FailedDeviceReply(NvDriver);
impl Service for FailedDeviceReply {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, r: &Request) -> Result<ServiceReply, ServiceError> {
        self.0.dispatch(r)
    }
    fn prepare(
        &mut self,
        r: &Request,
        c: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        let mut adjusted = r.clone();
        if r.command_id >= 100 {
            adjusted.command_id -= 100;
        }
        let mut reply = self.0.prepare(&adjusted, c)?;
        if r.command_id >= 100 {
            reply.data = vec![0; 300];
        }
        Ok(reply)
    }
}
#[test]
fn failed_nvmap_replies_rollback_fds_objects_free_and_output_bytes() {
    let mut r = Rig::device_rig(Box::new(FailedDeviceReply::default()));
    r.m.write(0x6000, b"/dev/nvmap").unwrap();
    assert!(matches!(
        r.call(&message(100, &[], Some((0x6000, 10)), None)),
        Err(KernelError::Ipc(_))
    ));
    let fd = r.open_device(b"/dev/nvmap", 0).0;
    assert_eq!(fd, 1);
    r.m.write(0x6000, &words(&[4096, 77])).unwrap();
    assert!(matches!(
        r.call(&message(
            101,
            &[fd, 0xc0080101],
            Some((0x6000, 8)),
            Some((0x6000, 8))
        )),
        Err(KernelError::Ipc(_))
    ));
    assert_eq!(r.m.read_u32(0x6004).unwrap(), 77);
    assert_eq!(r.ioctl(fd, 0xc0080101, &[4096, 0], 1), 0);
    assert_eq!(r.m.read_u32(0x6004).unwrap(), 1);
    r.m.write(0x6000, &words(&[1, 0, 77, 77, 77, 77])).unwrap();
    assert!(matches!(
        r.call(&message(
            101,
            &[fd, 0xc0180105],
            Some((0x6000, 24)),
            Some((0x6000, 24))
        )),
        Err(KernelError::Ipc(_))
    ));
    assert_eq!(r.m.read_u64(0x6008).unwrap(), 0x4d0000004d);
    assert_eq!(r.ioctl(fd, 0xc00c0109, &[1, 1, 0], 1), 0);
    assert_eq!(r.m.read_u32(0x6008).unwrap(), 4096);
    assert!(matches!(
        r.call(&message(102, &[fd], None, None)),
        Err(KernelError::Ipc(_))
    ));
    assert_eq!(r.close_fd(fd), 0);
}
#[test]
fn nvmap_fd_and_object_quotas_release_without_reusing_stale_ids() {
    let mut r = Rig::device_rig(Box::new(NvDriver::default()));
    for fd in 1..=16 {
        assert_eq!(r.open_device(b"/dev/nvmap", 0), (fd, 0));
    }
    assert_eq!(r.open_device(b"/dev/nvmap", 0), (0, 6));
    assert_eq!(r.close_fd(1), 0);
    assert_eq!(r.open_device(b"/dev/nvmap", 0), (17, 0));
    assert_eq!(r.ioctl(1, 0xc0080101, &[4096, 0], 1), 4);
    for handle in 1..=64 {
        assert_eq!(r.ioctl(17, 0xc0080101, &[4096, 0], 1), 0);
        assert_eq!(r.m.read_u32(0x6004).unwrap(), handle);
    }
    assert_eq!(r.ioctl(17, 0xc0080101, &[4096, 0], 1), 6);
    assert_eq!(r.ioctl(17, 0xc0180105, &[1, 0, 0, 0, 0, 0], 1), 0);
    assert_eq!(r.ioctl(17, 0xc0080101, &[4096, 0], 1), 0);
    assert_eq!(r.m.read_u32(0x6004).unwrap(), 65);
    // A separate device descriptor cannot use another descriptor's object.
    assert_eq!(r.ioctl(2, 0xc00c0109, &[65, 1, 0], 1), 4);
    assert_eq!(r.close_fd(17), 0);
    assert_eq!(r.ioctl(17, 0xc00c0109, &[65, 1, 0], 1), 4);
}

#[test]
fn nvmap_ioctl_writes_only_granted_b_and_rejects_mismatched_sizes_without_allocating() {
    let mut r = Rig::device_rig(Box::new(NvDriver::default()));
    let fd = r.open_device(b"/dev/nvmap", 0).0;
    let original = words(&[4096, 77]);
    r.m.write(0x6000, &original).unwrap();
    r.m.write(0x6100, &[0xaa; 12]).unwrap();
    r.call(&message(
        1,
        &[fd, 0xc0080101],
        Some((0x6000, 8)),
        Some((0x6100, 12)),
    ))
    .unwrap();
    assert_eq!(r.m.read_u32(0x2020).unwrap(), 4);
    let mut output = [0; 12];
    r.m.read(0x6100, &mut output).unwrap();
    assert_eq!(output, [0xaa; 12]);
    r.call(&message(
        1,
        &[fd, 0xc0080101],
        Some((0x6000, 8)),
        Some((0x6100, 8)),
    ))
    .unwrap();
    assert_eq!(r.m.read_u32(0x2020).unwrap(), 0);
    r.m.read(0x6100, &mut output).unwrap();
    assert_eq!(&output[..8], words(&[4096, 1]));
    assert_eq!(&output[8..], &[0xaa; 4]);
    let mut input = [0; 8];
    r.m.read(0x6000, &mut input).unwrap();
    assert_eq!(input.as_slice(), original);
}
