use emulator_a64::Cpu;
use emulator_kernel::*;
use emulator_memory::{AddressSpace, Permissions};

fn event() -> (Kernel, Cpu, AddressSpace, u32, u32) {
    let mut kernel = Kernel::default();
    let mut cpu = Cpu::default();
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, vec![0; 256], Permissions::READ_WRITE)
        .unwrap();
    kernel
        .dispatch(SVC_CREATE_EVENT, &mut cpu, &mut memory)
        .unwrap();
    let write = cpu.x[1] as u32;
    let read = cpu.x[2] as u32;
    assert_ne!(write, read);
    assert_eq!(cpu.x[0], 0);
    (kernel, cpu, memory, write, read)
}
fn invoke(k: &mut Kernel, cpu: &mut Cpu, m: &mut AddressSpace, svc: u16, handle: u32) -> u64 {
    cpu.x[0] = u64::from(handle);
    k.dispatch(svc, cpu, m).unwrap();
    cpu.x[0]
}
fn poll(k: &mut Kernel, cpu: &mut Cpu, m: &mut AddressSpace, handles: &[u32]) -> u64 {
    for (i, handle) in handles.iter().enumerate() {
        m.write_u32(0x1000 + i as u64 * 4, *handle).unwrap();
    }
    cpu.x[1] = 0x1000;
    cpu.x[2] = handles.len() as u64;
    cpu.x[3] = 0;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, cpu, m).unwrap();
    cpu.x[0]
}

#[test]
fn readable_and_writable_event_capabilities_preserve_manual_reset_state() {
    let (mut k, mut cpu, mut m, write, read) = event();
    assert_eq!(poll(&mut k, &mut cpu, &mut m, &[read]), RESULT_TIMEOUT);
    assert_eq!(cpu.x[1], 0x1000);
    assert_eq!(
        invoke(&mut k, &mut cpu, &mut m, SVC_SIGNAL_EVENT, read),
        RESULT_INVALID_HANDLE
    );
    assert_eq!(invoke(&mut k, &mut cpu, &mut m, SVC_SIGNAL_EVENT, write), 0);
    assert_eq!(poll(&mut k, &mut cpu, &mut m, &[read]), 0);
    assert_eq!(cpu.x[1], 0);
    assert_eq!(poll(&mut k, &mut cpu, &mut m, &[read]), 0);
    assert_eq!(invoke(&mut k, &mut cpu, &mut m, SVC_RESET_SIGNAL, read), 0);
    assert_eq!(
        invoke(&mut k, &mut cpu, &mut m, SVC_RESET_SIGNAL, read),
        RESULT_INVALID_STATE
    );
    assert_eq!(invoke(&mut k, &mut cpu, &mut m, SVC_SIGNAL_EVENT, write), 0);
    assert_eq!(invoke(&mut k, &mut cpu, &mut m, SVC_CLEAR_EVENT, write), 0);
    assert_eq!(poll(&mut k, &mut cpu, &mut m, &[read]), RESULT_TIMEOUT);
    assert_eq!(
        poll(&mut k, &mut cpu, &mut m, &[write]),
        RESULT_INVALID_HANDLE
    );
}

#[test]
fn closing_a_writer_keeps_the_readable_event_alive_and_waits_check_all_handles() {
    let (mut k, mut cpu, mut m, write, read) = event();
    invoke(&mut k, &mut cpu, &mut m, SVC_SIGNAL_EVENT, write);
    invoke(&mut k, &mut cpu, &mut m, SVC_CLOSE_HANDLE, write);
    assert_eq!(poll(&mut k, &mut cpu, &mut m, &[read]), 0);
    assert_eq!(
        poll(&mut k, &mut cpu, &mut m, &[read, 0xdead]),
        RESULT_INVALID_HANDLE
    );
    assert_eq!(
        invoke(&mut k, &mut cpu, &mut m, SVC_SIGNAL_EVENT, write),
        RESULT_INVALID_HANDLE
    );
    assert_eq!(invoke(&mut k, &mut cpu, &mut m, SVC_RESET_SIGNAL, read), 0);
    assert_eq!(poll(&mut k, &mut cpu, &mut m, &[read]), RESULT_TIMEOUT);
    invoke(&mut k, &mut cpu, &mut m, SVC_CLOSE_HANDLE, read);
    assert_eq!(
        poll(&mut k, &mut cpu, &mut m, &[read]),
        RESULT_INVALID_HANDLE
    );
}

#[test]
fn polling_validates_bounds_without_fabricating_blocking_timeout_behavior() {
    let (mut k, mut cpu, mut m, _write, read) = event();
    cpu.x[1] = u64::MAX;
    cpu.x[2] = 0;
    cpu.x[3] = 0;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut cpu, &mut m)
        .unwrap();
    assert_eq!(cpu.x[0], RESULT_TIMEOUT);
    cpu.x[2] = 1;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut cpu, &mut m)
        .unwrap();
    assert_eq!(cpu.x[0], RESULT_INVALID_ADDRESS);
    cpu.x[2] = 65;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut cpu, &mut m)
        .unwrap();
    assert_eq!(cpu.x[0], RESULT_TOO_MANY_HANDLES);
    m.write_u32(0x1000, read).unwrap();
    cpu.x[1] = 0x1000;
    cpu.x[2] = 1;
    cpu.x[3] = 100;
    let saved = cpu.clone();
    assert_eq!(
        k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut cpu, &mut m),
        Err(KernelError::UnsupportedWaitTimeout(100))
    );
    assert_eq!(cpu, saved);
}

#[test]
fn availability_hint_includes_event_creation_above_the_first_64_svc_bits() {
    let config = homebrew_config();
    let second = u64::from_le_bytes(config[64..72].try_into().unwrap());
    assert_eq!(
        second & (1 << (SVC_CREATE_EVENT - 64)),
        1 << (SVC_CREATE_EVENT - 64)
    );
}
