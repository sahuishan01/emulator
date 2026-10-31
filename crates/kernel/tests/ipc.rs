use emulator_a64::Cpu;
use emulator_ipc::{IpcError, Request};
use emulator_kernel::{
    service::{PreparedServiceEffect, Service, ServiceContext, ServiceError, ServiceReply},
    HandleError, Kernel, KernelError, RESULT_INVALID_HANDLE, SVC_CLOSE_HANDLE,
    SVC_CONNECT_TO_NAMED_PORT, SVC_SEND_SYNC_REQUEST,
};
use emulator_memory::{AddressSpace, BackingRange, MemoryView, Permissions};

#[derive(Clone, Debug, Default)]
struct Probe {
    value: u64,
}
#[derive(Debug)]
struct WriteEffect {
    view: MemoryView,
    ranges: Vec<BackingRange>,
    value: u64,
}
impl PreparedServiceEffect for WriteEffect {
    fn write_ranges(&self) -> &[BackingRange] {
        &self.ranges
    }
    fn commit(self: Box<Self>) {
        self.view
            .write(0, &self.value.to_le_bytes())
            .expect("prepared test pin");
    }
}
impl Service for Probe {
    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        let mut reply = self.dispatch(request)?;
        if matches!(request.command_id, 0 | 2 | 3) {
            let view = context.pin(0x3000, 8, Permissions::READ_WRITE)?;
            let ranges = view.backing_ranges().collect();
            reply.effect = Some(Box::new(WriteEffect {
                view,
                ranges,
                value: self.value,
            }));
        }
        Ok(reply)
    }
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => self.value += 1,
            1 => {}
            2 => {
                self.value += 1;
                return Ok(ServiceReply {
                    data: vec![0; 300],
                    ..ServiceReply::default()
                });
            }
            3 => {
                self.value += 1;
                return Ok(ServiceReply {
                    moved_objects: vec![Box::new(Self::default())],
                    ..ServiceReply::default()
                });
            }
            4 => {
                return Ok(ServiceReply {
                    data: request.pid.unwrap_or(u64::MAX).to_le_bytes().to_vec(),
                    ..ServiceReply::default()
                })
            }
            command => {
                return Err(ServiceError::UnsupportedCommand {
                    service: "probe:",
                    command,
                })
            }
        }
        Ok(ServiceReply {
            data: self.value.to_le_bytes().to_vec(),
            ..ServiceReply::default()
        })
    }
}

fn setup() -> (Kernel, Cpu, AddressSpace, u32) {
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, b"probe:\0".to_vec(), Permissions::READ)
        .unwrap();
    memory
        .map(0x2000, vec![0; 256], Permissions::READ_WRITE)
        .unwrap();
    memory
        .map(0x3000, vec![0x55; 8], Permissions::READ_WRITE)
        .unwrap();
    let mut kernel = Kernel::default();
    kernel
        .register_named_port("probe:", Box::new(Probe::default()))
        .unwrap();
    let mut cpu = Cpu::new(0, 0);
    cpu.tpidrro_el0 = 0x2000;
    cpu.x[1] = 0x1000;
    kernel
        .dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut cpu, &mut memory)
        .unwrap();
    let handle = cpu.x[1] as u32;
    (kernel, cpu, memory, handle)
}

fn packet(kind: u32, command: u32, pid: bool) -> [u8; 256] {
    let mut bytes = [0; 256];
    bytes[..4].copy_from_slice(&kind.to_le_bytes());
    bytes[4..8].copy_from_slice(&(8_u32 | if pid { 1 << 31 } else { 0 }).to_le_bytes());
    let cmif = if pid {
        bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[12..20].copy_from_slice(&0xdeadbeefdeadbeef_u64.to_le_bytes());
        32
    } else {
        16
    };
    bytes[cmif..cmif + 4].copy_from_slice(b"SFCI");
    bytes[cmif + 8..cmif + 12].copy_from_slice(&command.to_le_bytes());
    bytes
}
fn call(
    k: &mut Kernel,
    cpu: &mut Cpu,
    m: &mut AddressSpace,
    handle: u32,
    kind: u32,
    command: u32,
) -> Result<(), KernelError> {
    m.write(0x2000, &packet(kind, command, false)).unwrap();
    cpu.x[0] = u64::from(handle);
    k.dispatch(SVC_SEND_SYNC_REQUEST, cpu, m).map(|_| ())
}

#[test]
fn cloned_session_shares_state_and_survives_closing_the_original() {
    let (mut k, mut cpu, mut m, original) = setup();
    call(&mut k, &mut cpu, &mut m, original, 4, 0).unwrap();
    call(&mut k, &mut cpu, &mut m, original, 5, 2).unwrap();
    let alias = m.read_u32(0x200c).unwrap();
    assert_ne!(alias, original);
    cpu.x[0] = u64::from(original);
    k.dispatch(SVC_CLOSE_HANDLE, &mut cpu, &mut m).unwrap();
    call(&mut k, &mut cpu, &mut m, alias, 4, 0).unwrap();
    assert_eq!(m.read_u64(0x2020).unwrap(), 2);
    call(&mut k, &mut cpu, &mut m, original, 4, 1).unwrap();
    assert_eq!(cpu.x[0], RESULT_INVALID_HANDLE);
}

#[test]
fn unwritable_reply_cannot_mutate_the_service_or_request() {
    let (mut k, mut cpu, mut m, handle) = setup();
    let input = packet(4, 0, false);
    m.write(0x2000, &input).unwrap();
    m.protect(0x2000, 256, Permissions::READ).unwrap();
    cpu.x[0] = u64::from(handle);
    let saved = cpu.clone();
    assert!(matches!(
        k.dispatch(SVC_SEND_SYNC_REQUEST, &mut cpu, &mut m),
        Err(KernelError::Memory(_))
    ));
    assert_eq!(cpu, saved);
    let mut actual = [0; 256];
    m.read(0x2000, &mut actual).unwrap();
    assert_eq!(actual, input);
    assert!(k.ipc_records().is_empty());
    m.protect(0x2000, 256, Permissions::READ_WRITE).unwrap();
    call(&mut k, &mut cpu, &mut m, handle, 4, 1).unwrap();
    assert_eq!(m.read_u64(0x2020).unwrap(), 0);
}

#[test]
fn oversized_reply_rolls_back_service_state_and_buffer() {
    let (mut k, mut cpu, mut m, handle) = setup();
    assert!(matches!(
        call(&mut k, &mut cpu, &mut m, handle, 4, 2),
        Err(KernelError::Ipc(IpcError::TooLarge))
    ));
    let mut actual = [0; 256];
    m.read(0x2000, &mut actual).unwrap();
    assert_eq!(actual, packet(4, 2, false));
    assert_eq!(k.live_handle_count(), 2);
    assert_eq!(m.read_u64(0x3000).unwrap(), 0x5555555555555555);
    call(&mut k, &mut cpu, &mut m, handle, 4, 1).unwrap();
    assert_eq!(m.read_u64(0x2020).unwrap(), 0);
}

#[test]
fn handle_exhaustion_is_atomic_and_closed_handles_are_not_reused() {
    let (mut k, mut cpu, mut m, handle) = setup();
    for _ in 0..254 {
        cpu.x[1] = 0x1000;
        k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut cpu, &mut m)
            .unwrap();
    }
    assert_eq!(k.live_handle_count(), 256);
    let last = cpu.x[1] as u32;
    assert_eq!(
        call(&mut k, &mut cpu, &mut m, handle, 4, 3),
        Err(KernelError::Handle(HandleError::Limit))
    );
    let mut actual = [0; 256];
    m.read(0x2000, &mut actual).unwrap();
    assert_eq!(actual, packet(4, 3, false));
    assert_eq!(k.live_handle_count(), 256);
    assert_eq!(m.read_u64(0x3000).unwrap(), 0x5555555555555555);
    cpu.x[0] = u64::from(last);
    k.dispatch(SVC_CLOSE_HANDLE, &mut cpu, &mut m).unwrap();
    let before = cpu.clone();
    assert_eq!(
        k.dispatch(emulator_kernel::SVC_CREATE_EVENT, &mut cpu, &mut m),
        Err(KernelError::Handle(HandleError::Limit))
    );
    assert_eq!(cpu, before);
    assert_eq!(k.live_handle_count(), 255);
    call(&mut k, &mut cpu, &mut m, handle, 4, 1).unwrap();
    assert_eq!(m.read_u64(0x2020).unwrap(), 0);
    cpu.x[1] = 0x1000;
    k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut cpu, &mut m)
        .unwrap();
    assert!(cpu.x[1] > u64::from(last));
    call(&mut k, &mut cpu, &mut m, last, 4, 1).unwrap();
    assert_eq!(cpu.x[0], RESULT_INVALID_HANDLE);
}

#[test]
fn sender_pid_is_replaced_and_unsupported_commands_do_not_succeed() {
    let (mut k, mut cpu, mut m, handle) = setup();
    m.write(0x2000, &packet(4, 4, true)).unwrap();
    cpu.x[0] = u64::from(handle);
    k.dispatch(SVC_SEND_SYNC_REQUEST, &mut cpu, &mut m).unwrap();
    assert_eq!(m.read_u64(0x2020).unwrap(), 1);
    assert!(matches!(
        call(&mut k, &mut cpu, &mut m, handle, 4, 999),
        Err(KernelError::Service(
            ServiceError::UnsupportedCommand { .. }
        ))
    ));
    assert!(matches!(
        call(&mut k, &mut cpu, &mut m, handle, 5, 999),
        Err(KernelError::UnsupportedControl(999))
    ));
}

#[test]
fn valid_reply_commits_the_prepared_effect_and_service_state_together() {
    let (mut k, mut cpu, mut m, handle) = setup();
    call(&mut k, &mut cpu, &mut m, handle, 4, 0).unwrap();
    assert_eq!(m.read_u64(0x3000).unwrap(), 1);
    call(&mut k, &mut cpu, &mut m, handle, 4, 1).unwrap();
    assert_eq!(m.read_u64(0x2020).unwrap(), 1);
}
