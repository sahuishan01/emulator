use emulator_a64::Cpu;
use emulator_kernel::{
    service::ServiceError, Kernel, KernelError, SVC_CONNECT_TO_NAMED_PORT, SVC_SEND_SYNC_REQUEST,
};
use emulator_memory::{AddressSpace, Permissions};
use emulator_services::DiagnosticGpu;
const GPU: u64 = 0x2_0000_0000;
struct Rig {
    kernel: Kernel,
    cpu: Cpu,
    memory: AddressSpace,
    handle: u32,
}
impl Rig {
    fn new() -> Self {
        let mut memory = AddressSpace::with_limit(16384);
        memory
            .map(0x1000, b"gpu:\0".to_vec(), Permissions::READ)
            .unwrap();
        memory
            .map(0x2000, vec![0; 256], Permissions::READ_WRITE)
            .unwrap();
        memory
            .map(0x3000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        let methods = [
            (0, 0xb0b5),
            (0x400, 2),
            (0x404, 0),
            (0x408, 2),
            (0x40c, 16),
            (0x418, 8),
            (0x300, 0x182),
        ];
        let commands: Vec<u8> = methods
            .into_iter()
            .flat_map(|(offset, data)| [0x2001_0000_u32 | (offset / 4), data])
            .flat_map(u32::to_le_bytes)
            .collect();
        memory.write(0x3000, &commands).unwrap();
        memory.write_u64(0x3100, 0x0000_3801_0000_1000).unwrap(); // 14 words, 40-bit command VA.
        memory.write_u64(0x3200, 42).unwrap();
        memory.write_u64(0x3210, 99).unwrap();
        let mut kernel = Kernel::default();
        kernel
            .register_named_port("gpu:", Box::new(DiagnosticGpu::default()))
            .unwrap();
        let mut cpu = Cpu::new(0, 0);
        cpu.tpidrro_el0 = 0x2000;
        cpu.x[1] = 0x1000;
        kernel
            .dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut cpu, &mut memory)
            .unwrap();
        let handle = cpu.x[1] as u32;
        Self {
            kernel,
            cpu,
            memory,
            handle,
        }
    }
    fn args() -> [u64; 8] {
        [0x3000, 0x1_0000_1000, 56, 0x3200, GPU, 32, 0x3100, 8]
    }
    fn call(&mut self, command: u32, args: &[u64]) -> Result<(), KernelError> {
        let mut request = [0; 256];
        request[0..4].copy_from_slice(&4_u32.to_le_bytes());
        request[4..8].copy_from_slice(&(8 + args.len() as u32 * 2).to_le_bytes());
        request[16..20].copy_from_slice(b"SFCI");
        request[24..28].copy_from_slice(&command.to_le_bytes());
        for (index, value) in args.iter().enumerate() {
            request[32 + index * 8..40 + index * 8].copy_from_slice(&value.to_le_bytes());
        }
        self.memory.write(0x2000, &request).unwrap();
        self.cpu.x[0] = u64::from(self.handle);
        self.kernel
            .dispatch(SVC_SEND_SYNC_REQUEST, &mut self.cpu, &mut self.memory)
            .map(|_| ())
    }
    fn count(&mut self) -> u64 {
        self.call(1, &[]).unwrap();
        self.memory.read_u64(0x2020).unwrap()
    }
}
#[test]
fn guest_ipc_prepares_commits_and_reports_actual_dma_bytes() {
    let mut r = Rig::new();
    r.call(0, &Rig::args()).unwrap();
    assert_eq!(r.memory.read_u64(0x3210).unwrap(), 42);
    for (offset, expected) in [(0x2020, 7), (0x2028, 1), (0x2030, 8), (0x2038, 1)] {
        assert_eq!(r.memory.read_u64(offset).unwrap(), expected);
    }
    assert_eq!(r.count(), 1);
    r.memory.write_u64(0x3200, 123).unwrap();
    r.call(0, &Rig::args()).unwrap();
    assert_eq!(r.memory.read_u64(0x3210).unwrap(), 123);
    assert_eq!(r.count(), 2);
}
#[test]
fn late_bad_command_cannot_copy_or_increment_completed_count() {
    let mut r = Rig::new();
    r.memory.write_u32(0x3038, 0x2001_0240).unwrap();
    r.memory.write_u32(0x303c, 0).unwrap();
    r.memory.write_u64(0x3100, 0x0000_4001_0000_1000).unwrap();
    let mut args = Rig::args();
    args[2] = 64;
    assert!(matches!(
        r.call(0, &args),
        Err(KernelError::Service(ServiceError::DeviceFault(_)))
    ));
    assert_eq!(r.memory.read_u64(0x3210).unwrap(), 99);
    assert_eq!(r.count(), 0);
}
#[test]
fn malformed_ranges_permissions_and_budgets_leave_data_and_state_untouched() {
    for (index, value) in [
        (2, 0),
        (2, 65540),
        (2, 55),
        (5, 4 * 1024 * 1024 + 1),
        (7, 7),
        (7, 2056),
        (0, u64::MAX),
        (1, 1 << 40),
        (4, 0x1_0000_1000),
    ] {
        let mut r = Rig::new();
        let mut args = Rig::args();
        args[index] = value;
        let resident = r.memory.resident_bytes();
        assert!(r.call(0, &args).is_err(), "argument {index}={value}");
        assert_eq!(r.memory.resident_bytes(), resident);
        assert_eq!(r.memory.read_u64(0x3210).unwrap(), 99);
        assert_eq!(r.count(), 0);
    }
    let mut r = Rig::new();
    r.memory.protect(0x3200, 32, Permissions::READ).unwrap();
    assert!(r.call(0, &Rig::args()).is_err());
    assert_eq!(r.count(), 0);
}
#[test]
fn dma_destination_must_not_alias_tls_reply_even_at_another_cpu_address() {
    let mut r = Rig::new();
    let view = r.memory.pin(0x2000, 256, Permissions::READ_WRITE).unwrap();
    r.memory
        .map_alias(0x4000, &view, Permissions::READ_WRITE)
        .unwrap();
    drop(view);
    let mut args = Rig::args();
    args[3] = 0x4000;
    args[5] = 256;
    assert_eq!(
        r.call(0, &args),
        Err(KernelError::Service(ServiceError::InvalidEffect))
    );
    assert_eq!(
        r.memory.read_u32(0x2010).unwrap(),
        u32::from_le_bytes(*b"SFCI")
    );
    assert_eq!(r.count(), 0);
}
#[test]
fn read_only_reply_and_unknown_commands_cannot_submit_work() {
    let mut r = Rig::new();
    r.memory.protect(0x2000, 256, Permissions::READ).unwrap();
    r.cpu.x[0] = u64::from(r.handle);
    assert!(r
        .kernel
        .dispatch(SVC_SEND_SYNC_REQUEST, &mut r.cpu, &mut r.memory)
        .is_err());
    r.memory
        .protect(0x2000, 256, Permissions::READ_WRITE)
        .unwrap();
    assert_eq!(r.count(), 0);
    assert!(r.call(99, &[]).is_err());
    assert!(r.call(0, &[]).is_err());
    assert_eq!(r.count(), 0);
    assert_eq!(r.memory.read_u64(0x3210).unwrap(), 99);
}
