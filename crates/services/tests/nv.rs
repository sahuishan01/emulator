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
    assert!(matches!(
        r.request(0, 4096, CURRENT_PROCESS_HANDLE, r.tmem),
        Err(KernelError::Service(ServiceError::UnsupportedCommand {
            service: "nvdrv",
            command: 0
        }))
    ));
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
        r.request(0, 4096, CURRENT_PROCESS_HANDLE, r.tmem),
        Err(KernelError::Service(ServiceError::InvalidCopyHandleCount(
            2
        )))
    );
    r.close(r.tmem);
    assert_eq!(r.k.live_transfer_memory_count(), 0);
    assert_eq!(r.m.read_u64(0x4000).unwrap(), 42);
}
