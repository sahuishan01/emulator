use emulator_a64::Cpu;
use emulator_kernel::*;
use emulator_memory::{AddressSpace, Permissions};

fn setup() -> (Kernel, Cpu, AddressSpace) {
    let mut m = AddressSpace::default();
    m.map(0x1000, vec![0; 4096], Permissions::READ_EXECUTE)
        .unwrap();
    m.map(0x4000, vec![0; 4096], Permissions::READ_WRITE)
        .unwrap();
    (Kernel::default(), Cpu::new(0x1000, 0x5000), m)
}
fn create(k: &mut Kernel, c: &mut Cpu, m: &mut AddressSpace, priority: u64) -> u32 {
    c.x[1] = 0x1000;
    c.x[2] = 42;
    c.x[3] = 0x5000;
    c.x[4] = priority;
    c.x[5] = 0;
    k.dispatch(SVC_CREATE_THREAD, c, m).unwrap();
    assert_eq!(c.x[0], 0);
    c.x[1] as u32
}
fn call(k: &mut Kernel, c: &mut Cpu, m: &mut AddressSpace, svc: u16, argument: u64) {
    c.x[0] = argument;
    k.dispatch(svc, c, m).unwrap();
}

#[test]
fn threads_validate_entry_stack_priority_core_and_publish_atomically() {
    let (mut k, mut c, mut m) = setup();
    for (register, bad) in [
        (1, 0x1001),
        (1, 0x4000),
        (3, 0x5001),
        (3, 0x1000),
        (4, 64),
        (5, 1),
    ] {
        c.x[1] = 0x1000;
        c.x[2] = 42;
        c.x[3] = 0x5000;
        c.x[4] = 44;
        c.x[5] = 0;
        c.x[register] = bad;
        let before = c.clone();
        let bytes = m.mapped_bytes();
        assert!(k.dispatch(SVC_CREATE_THREAD, &mut c, &mut m).is_err());
        assert_eq!(c, before);
        assert_eq!(m.mapped_bytes(), bytes);
        assert_eq!(k.live_handle_count(), 1);
        assert_eq!(k.scheduler_stats().threads_created, 1);
    }
    m.map(
        THREAD_TLS_ARENA + 4096,
        vec![0; 4096],
        Permissions::READ_WRITE,
    )
    .unwrap();
    c.x[1] = 0x1000;
    c.x[2] = 42;
    c.x[3] = 0x5000;
    c.x[4] = 44;
    c.x[5] = 0;
    assert!(matches!(
        k.dispatch(SVC_CREATE_THREAD, &mut c, &mut m),
        Err(KernelError::Memory(_))
    ));
    assert_eq!(k.live_handle_count(), 1);
    assert_eq!(k.scheduler_stats().threads_created, 1);
    m.unmap(THREAD_TLS_ARENA + 4096, 4096).unwrap();
    let handle = create(&mut k, &mut c, &mut m, 44);
    assert_eq!(handle, 0x101);
    for _ in 2..MAX_THREADS {
        create(&mut k, &mut c, &mut m, 44);
    }
    assert_eq!(k.scheduler_stats().threads_created, MAX_THREADS);
    assert!(matches!(
        k.dispatch(SVC_CREATE_THREAD, &mut c, &mut m),
        Err(KernelError::Scheduler(SchedulerError::ThreadLimit))
    ));
}

#[test]
fn priority_preemption_preserves_registers_flags_stack_and_guest_tls() {
    let (mut k, mut c, mut m) = setup();
    let child = create(&mut k, &mut c, &mut m, 10);
    c.x[19] = 77;
    c.nzcv.zero = true;
    c.tpidrro_el0 = THREAD_TLS_ARENA + 0x200;
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child.into());
    let main = c.clone();
    assert_eq!(k.prepare_execution(&mut c), ScheduleOutcome::Ready);
    assert_eq!(k.current_thread_id(), 1);
    assert_eq!(c.x[0], 42);
    assert_eq!(c.pc, 0x1000);
    assert_eq!(c.sp, 0x5000);
    assert_eq!(c.tpidrro_el0, THREAD_TLS_ARENA + 0x1200);
    assert_eq!(c.x[19], 0);
    assert!(!c.nzcv.zero);
    c.x[19] = 999;
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 100);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    assert_eq!(c, main);
    assert_eq!(k.scheduler_stats().virtual_time_ns, 0);
}

#[test]
fn equal_priority_threads_preempt_after_quantum_and_yield_without_host_sleeps() {
    let (mut k, mut c, mut m) = setup();
    let child = create(&mut k, &mut c, &mut m, 44);
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child.into());
    for _ in 0..THREAD_QUANTUM - 1 {
        k.instruction_completed().unwrap();
        k.prepare_execution(&mut c);
    }
    assert_eq!(k.current_thread_id(), 0);
    k.instruction_completed().unwrap();
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 0);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    let before = k.scheduler_stats();
    c.x[0] = u64::MAX;
    assert!(matches!(
        k.dispatch(SVC_SLEEP_THREAD, &mut c, &mut m),
        Err(KernelError::Scheduler(SchedulerError::UnsupportedSleep(-1)))
    ));
    assert_eq!(k.scheduler_stats(), before);
}

#[test]
fn blocked_wait_retains_object_when_another_thread_closes_its_handle() {
    let (mut k, mut c, mut m) = setup();
    let child = create(&mut k, &mut c, &mut m, 44);
    k.dispatch(SVC_CREATE_EVENT, &mut c, &mut m).unwrap();
    let write = c.x[1];
    let read = c.x[2];
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child.into());
    m.write_u32(0x4000, read as u32).unwrap();
    c.x[1] = 0x4000;
    c.x[2] = 1;
    c.x[3] = u64::MAX;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut c, &mut m)
        .unwrap();
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    call(&mut k, &mut c, &mut m, SVC_CLOSE_HANDLE, read);
    m.write_u32(0x4000, 0xffff).unwrap(); // Never re-read guest handle arrays while blocked.
    call(&mut k, &mut c, &mut m, SVC_SIGNAL_EVENT, write);
    k.prepare_execution(&mut c);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 0);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    assert_eq!(c.x[0], 0);
    assert_eq!(c.x[1], 0);
    assert_eq!(k.scheduler_stats().blocking_waits, 1);
}

#[test]
fn finite_wait_expires_while_another_thread_is_running_and_preserves_index() {
    let (mut k, mut c, mut m) = setup();
    let child = create(&mut k, &mut c, &mut m, 44);
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child.into());
    c.x[1] = u64::MAX;
    c.x[2] = 0;
    c.x[3] = 3;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut c, &mut m)
        .unwrap();
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    for _ in 0..3 {
        k.instruction_completed().unwrap();
        k.prepare_execution(&mut c);
    }
    assert_eq!(k.scheduler_stats().timeouts, 1);
    assert_eq!(k.scheduler_stats().virtual_time_ns, 3);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 0);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    assert_eq!(c.x[0], RESULT_TIMEOUT);
    assert_eq!(c.x[1], u64::MAX);
}

#[test]
fn joining_thread_exit_and_closing_active_handle_preserve_thread_lifetime() {
    let (mut k, mut c, mut m) = setup();
    let child = create(&mut k, &mut c, &mut m, 44);
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child.into());
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, child.into());
    assert_eq!(c.x[0], RESULT_INVALID_STATE);
    m.write_u32(0x4000, child).unwrap();
    c.x[1] = 0x4000;
    c.x[2] = 1;
    c.x[3] = u64::MAX;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut c, &mut m)
        .unwrap();
    k.prepare_execution(&mut c);
    call(&mut k, &mut c, &mut m, SVC_CLOSE_HANDLE, child.into());
    assert_eq!(
        k.dispatch(SVC_EXIT_THREAD, &mut c, &mut m).unwrap(),
        KernelEvent::Continue
    );
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 0);
    assert_eq!(c.x[0], 0);
    assert_eq!(c.x[1], 0);
    assert_eq!(
        k.dispatch(SVC_EXIT_THREAD, &mut c, &mut m).unwrap(),
        KernelEvent::ProcessExited
    );
    assert_eq!(k.live_handle_count(), 0);
}

#[test]
fn indefinite_wait_is_bounded_blocked_outcome_and_deadline_overflow_is_explicit() {
    let (mut k, mut c, mut m) = setup();
    c.x[1] = u64::MAX;
    c.x[2] = 0;
    c.x[3] = u64::MAX;
    k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut c, &mut m)
        .unwrap();
    assert_eq!(k.prepare_execution(&mut c), ScheduleOutcome::Blocked);
    assert_eq!(k.prepare_execution(&mut c), ScheduleOutcome::Blocked);
    assert_eq!(k.scheduler_stats().virtual_time_ns, 0);
    let (mut k, mut c, mut m) = setup();
    for _ in 0..2 {
        call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, i64::MAX as u64);
        assert_eq!(k.prepare_execution(&mut c), ScheduleOutcome::Ready);
    }
    let before = k.scheduler_stats();
    c.x[0] = 2;
    assert_eq!(
        k.dispatch(SVC_SLEEP_THREAD, &mut c, &mut m),
        Err(KernelError::Scheduler(SchedulerError::ClockOverflow))
    );
    assert_eq!(k.scheduler_stats(), before);
}

#[test]
fn a_manual_reset_event_wakes_every_existing_waiter_before_a_later_clear() {
    let (mut k, mut c, mut m) = setup();
    let first = create(&mut k, &mut c, &mut m, 44);
    let second = create(&mut k, &mut c, &mut m, 44);
    k.dispatch(SVC_CREATE_EVENT, &mut c, &mut m).unwrap();
    let write = c.x[1];
    let read = c.x[2];
    m.write_u32(0x4000, read as u32).unwrap();
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, first.into());
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, second.into());
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 0);
    k.prepare_execution(&mut c);
    assert_eq!(k.current_thread_id(), 1);
    for id in [1, 2] {
        assert_eq!(k.current_thread_id(), id);
        c.x[1] = 0x4000;
        c.x[2] = 1;
        c.x[3] = u64::MAX;
        k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut c, &mut m)
            .unwrap();
        k.prepare_execution(&mut c);
    }
    assert_eq!(k.current_thread_id(), 0);
    call(&mut k, &mut c, &mut m, SVC_SIGNAL_EVENT, write);
    k.prepare_execution(&mut c);
    call(&mut k, &mut c, &mut m, SVC_CLEAR_EVENT, write);
    call(&mut k, &mut c, &mut m, SVC_SLEEP_THREAD, 0);
    k.prepare_execution(&mut c);
    for id in [1, 2] {
        assert_eq!(k.current_thread_id(), id);
        assert_eq!(c.x[0], 0);
        assert_eq!(c.x[1], 0);
        k.dispatch(SVC_EXIT_THREAD, &mut c, &mut m).unwrap();
        k.prepare_execution(&mut c);
    }
    assert_eq!(k.current_thread_id(), 0);
    assert_eq!(k.scheduler_stats().blocking_waits, 2);
    assert_eq!(k.scheduler_stats().timeouts, 0);
}

#[test]
fn thread_creation_at_handle_capacity_does_not_map_tls_or_consume_identity() {
    let (mut k, mut c, mut m) = setup();
    // Main handle plus 127 two-handle events plus one named-port session = 256.
    #[derive(Clone, Debug)]
    struct Empty;
    impl emulator_kernel::service::Service for Empty {
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
    k.register_named_port("test", Box::new(Empty)).unwrap();
    for _ in 0..127 {
        k.dispatch(SVC_CREATE_EVENT, &mut c, &mut m).unwrap();
    }
    m.write(0x4000, b"test\0").unwrap();
    c.x[1] = 0x4000;
    k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m)
        .unwrap();
    let handle = c.x[1];
    let bytes = m.mapped_bytes();
    c.x[1] = 0x1000;
    c.x[2] = 42;
    c.x[3] = 0x5000;
    c.x[4] = 44;
    c.x[5] = 0;
    assert!(matches!(
        k.dispatch(SVC_CREATE_THREAD, &mut c, &mut m),
        Err(KernelError::Handle(HandleError::Limit))
    ));
    assert_eq!(m.mapped_bytes(), bytes);
    assert_eq!(k.scheduler_stats().threads_created, 1);
    assert!(m.query(THREAD_TLS_ARENA + 4096).is_none());
    call(&mut k, &mut c, &mut m, SVC_CLOSE_HANDLE, handle);
    create(&mut k, &mut c, &mut m, 44);
    assert_eq!(k.scheduler_stats().threads_created, 2);
    call(&mut k, &mut c, &mut m, SVC_START_THREAD, 0xffff);
    assert_eq!(c.x[0], RESULT_INVALID_HANDLE);
}
