use emulator_a64::Cpu;
use emulator_kernel::*;
use emulator_memory::{AddressSpace, Permissions};

fn setup() -> (Kernel, Cpu, AddressSpace, u32, u32, u32) {
    let mut k = Kernel::default();
    let mut c = Cpu::new(0x1000, 0x5000);
    let mut m = AddressSpace::default();
    m.map(0x1000, vec![0; 4096], Permissions::READ_EXECUTE)
        .unwrap();
    m.map(0x4000, vec![0; 4096], Permissions::READ_WRITE)
        .unwrap();
    c.x[1] = 0x1000;
    c.x[2] = 7;
    c.x[3] = 0x5000;
    c.x[4] = 44;
    c.x[5] = 0;
    k.dispatch(SVC_CREATE_THREAD, &mut c, &mut m).unwrap();
    let child = c.x[1] as u32;
    k.dispatch(SVC_CREATE_EVENT, &mut c, &mut m).unwrap();
    let write = c.x[1] as u32;
    let read = c.x[2] as u32;
    m.write_u32(0x4000, read).unwrap();
    (k, c, m, child, write, read)
}
fn call(k: &mut Kernel, c: &mut Cpu, m: &mut AddressSpace, svc: u16, argument: u32) {
    c.x[0] = argument.into();
    k.dispatch(svc, c, m).unwrap();
}
fn wait(k: &mut Kernel, c: &mut Cpu, m: &mut AddressSpace, timeout: u64) {
    c.x[1] = 0x4000;
    c.x[2] = 1;
    c.x[3] = timeout;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, c, m).unwrap();
}

#[test]
fn cancellation_interrupts_a_blocked_wait_and_removes_its_deadline_and_objects() {
    let (mut k, mut c, mut m, child, write, read) = setup();
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child);
    wait(&mut k, &mut c, &mut m, 100);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    call(
        &mut k,
        &mut c,
        &mut m,
        SVC_CANCEL_SYNCHRONIZATION,
        MAIN_THREAD_HANDLE as u32,
    );
    assert_eq!(c.x[0], 0);
    call(&mut k, &mut c, &mut m, SVC_SIGNAL_EVENT, write);
    call(&mut k, &mut c, &mut m, SVC_CLOSE_HANDLE, read);
    k.dispatch(SVC_EXIT_THREAD, &mut c, &mut m).unwrap();
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    assert_eq!(c.x[0], RESULT_INTERRUPTED);
    assert_eq!(c.x[1], 0x4000);
    assert_eq!(k.scheduler_stats().cancelled_waits, 1);
    assert_eq!(k.scheduler_stats().timeouts, 0);
    assert_eq!(k.scheduler_stats().virtual_time_ns, 0);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 200);
    k.prepare_execution(&mut c);
    assert_eq!(k.scheduler_stats().timeouts, 0);
    assert_eq!(k.scheduler_stats().cancelled_waits, 1);
}

#[test]
fn pending_cancellation_coalesces_and_survives_invalid_immediate_and_poll_results() {
    let (mut k, mut c, mut m, _, write, read) = setup();
    for _ in 0..2 {
        call(
            &mut k,
            &mut c,
            &mut m,
            SVC_CANCEL_SYNCHRONIZATION,
            MAIN_THREAD_HANDLE as u32,
        );
    }
    c.x[1] = u64::MAX;
    c.x[2] = 1;
    c.x[3] = 20;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut c, &mut m)
        .unwrap();
    assert_eq!(c.x[0], RESULT_INVALID_ADDRESS);
    m.write_u32(0x4000, 0xffff).unwrap();
    wait(&mut k, &mut c, &mut m, 20);
    assert_eq!(c.x[0], RESULT_INVALID_HANDLE);
    m.write_u32(0x4000, read).unwrap();
    wait(&mut k, &mut c, &mut m, 0);
    assert_eq!(c.x[0], RESULT_TIMEOUT);
    call(&mut k, &mut c, &mut m, SVC_SIGNAL_EVENT, write);
    wait(&mut k, &mut c, &mut m, 20);
    assert_eq!(c.x[0], 0);
    assert_eq!(c.x[1], 0);
    call(&mut k, &mut c, &mut m, SVC_CLEAR_EVENT, read);
    assert_eq!(c.x[0], 0);
    wait(&mut k, &mut c, &mut m, 20);
    assert_eq!(c.x[0], RESULT_INTERRUPTED);
    assert_eq!(c.x[1], 0x4000);
    assert_eq!(k.scheduler_stats().cancelled_waits, 1);
    wait(&mut k, &mut c, &mut m, 20);
    k.prepare_execution(&mut c);
    assert_eq!(c.x[0], RESULT_TIMEOUT);
    assert_eq!(k.scheduler_stats().cancelled_waits, 1);
    assert_eq!(k.scheduler_stats().timeouts, 1);
}

#[test]
fn cancellation_of_a_sleeping_thread_does_not_wake_it_or_affect_another_thread() {
    let (mut k, mut c, mut m, child, _, _) = setup();
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 0);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 100);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    call(&mut k, &mut c, &mut m, SVC_CANCEL_SYNCHRONIZATION, child);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    wait(&mut k, &mut c, &mut m, 20);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    assert_eq!(c.x[0], RESULT_TIMEOUT);
    assert_eq!(k.scheduler_stats().virtual_time_ns, 20);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 200);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    assert_eq!(k.scheduler_stats().virtual_time_ns, 100);
    wait(&mut k, &mut c, &mut m, u64::MAX);
    assert_eq!(c.x[0], RESULT_INTERRUPTED);
    assert_eq!(c.x[1], 0x4000);
    assert_eq!(k.scheduler_stats().cancelled_waits, 1);
}

#[test]
fn cancellation_after_a_signal_preserves_completed_result_and_targets_the_next_wait() {
    let (mut k, mut c, mut m, child, write, read) = setup();
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child);
    wait(&mut k, &mut c, &mut m, u64::MAX);
    k.prepare_execution(&mut c);
    call(&mut k, &mut c, &mut m, SVC_SIGNAL_EVENT, write);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    call(
        &mut k,
        &mut c,
        &mut m,
        SVC_CANCEL_SYNCHRONIZATION,
        MAIN_THREAD_HANDLE as u32,
    );
    k.dispatch(SVC_EXIT_THREAD, &mut c, &mut m).unwrap();
    k.prepare_execution(&mut c);
    assert_eq!(c.x[0], 0);
    assert_eq!(c.x[1], 0);
    call(&mut k, &mut c, &mut m, SVC_CLEAR_EVENT, read);
    wait(&mut k, &mut c, &mut m, 100);
    assert_eq!(c.x[0], RESULT_INTERRUPTED);
    assert_eq!(k.scheduler_stats().cancelled_waits, 1);
}

#[test]
fn cancelled_targets_must_be_thread_handles_and_readable_event_clear_is_supported() {
    let (mut k, mut c, mut m, child, write, read) = setup();
    let before = k.scheduler_stats();
    for handle in [0xffff, write, read] {
        call(&mut k, &mut c, &mut m, SVC_CANCEL_SYNCHRONIZATION, handle);
        assert_eq!(c.x[0], RESULT_INVALID_HANDLE);
        assert_eq!(k.scheduler_stats(), before);
    }
    call(&mut k, &mut c, &mut m, SVC_CLOSE_HANDLE, child);
    call(&mut k, &mut c, &mut m, SVC_CANCEL_SYNCHRONIZATION, child);
    assert_eq!(c.x[0], RESULT_INVALID_HANDLE);
    call(&mut k, &mut c, &mut m, SVC_SIGNAL_EVENT, write);
    call(&mut k, &mut c, &mut m, SVC_CLEAR_EVENT, read);
    assert_eq!(c.x[0], 0);
    wait(&mut k, &mut c, &mut m, 0);
    assert_eq!(c.x[0], RESULT_TIMEOUT);
    call(&mut k, &mut c, &mut m, SVC_SIGNAL_EVENT, read);
    assert_eq!(c.x[0], RESULT_INVALID_HANDLE);
    let bytes = homebrew_config();
    let bits = u64::from_le_bytes(bytes[56..64].try_into().unwrap());
    assert_ne!(bits & (1 << SVC_CANCEL_SYNCHRONIZATION), 0);
}
