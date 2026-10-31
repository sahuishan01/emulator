use emulator_kernel::KernelError;
use emulator_session::{
    RunOutcome, Session, SessionError, SessionState, CONFIG_BASE, LOAD_BASE, STACK_BASE, STACK_SIZE,
};

const HELLO: &[u8] = include_bytes!("../../../examples/homebrew/hello.nro");

#[test]
fn owned_homebrew_connects_initializes_sends_messages_and_closes_handles() {
    let mut session =
        Session::from_nro(include_bytes!("../../../examples/homebrew/ipc-counter.nro")).unwrap();
    for _ in 0..2000 {
        if !session.debug_records().is_empty() {
            break;
        }
        assert_eq!(session.run(1).unwrap(), RunOutcome::BudgetExhausted);
    }
    assert_eq!(
        session.debug_records(),
        &[b"Guest IPC counter reached 42; handles closed.\n".to_vec()]
    );
    assert_eq!(session.kernel().live_handle_count(), 1); // Main thread only.
    assert_eq!(session.memory().read_u64(LOAD_BASE + 0x2000).unwrap(), 42);
    let commands: Vec<_> = session
        .kernel()
        .ipc_records()
        .iter()
        .map(|r| r.command_id)
        .collect();
    assert_eq!(commands, [0, 1, 0, 0, 1]);
    assert_eq!(session.run(10).unwrap(), RunOutcome::Exited);
    assert_eq!(session.kernel().live_handle_count(), 0);
}

#[test]
fn owned_guest_clone_survives_original_close_and_events_have_real_state() {
    for (bytes, expected, value, offset) in [
        (
            include_bytes!("../../../examples/homebrew/ipc-clone.nro").as_slice(),
            "Guest IPC clone retained counter=43 after close.\n",
            43,
            0x2000,
        ),
        (
            include_bytes!("../../../examples/homebrew/events.nro").as_slice(),
            "Guest events signaled, waited, reset and closed.\n",
            7,
            0x2008,
        ),
    ] {
        let mut session = Session::from_nro(bytes).unwrap();
        assert_eq!(session.run(5000).unwrap(), RunOutcome::Exited);
        assert_eq!(session.debug_records(), &[expected.as_bytes().to_vec()]);
        assert_eq!(
            session.memory().read_u64(LOAD_BASE + offset).unwrap(),
            value
        );
        assert_eq!(session.kernel().live_handle_count(), 0);
    }
}

#[test]
fn original_homebrew_prints_and_exits_through_real_svc_dispatch() {
    let mut session = Session::from_nro(HELLO).unwrap();
    assert_eq!(session.state(), SessionState::Loaded);
    assert_eq!(session.cpu().x[0], CONFIG_BASE);
    assert_eq!(session.cpu().x[1], u64::MAX);
    assert_eq!(session.cpu().sp, STACK_BASE + STACK_SIZE as u64);
    assert_eq!(session.memory().read_u32(CONFIG_BASE).unwrap(), 1);
    assert_eq!(session.run(100).unwrap(), RunOutcome::Exited);
    assert_eq!(session.state(), SessionState::Exited);
    assert_eq!(session.instructions(), 5);
    assert_eq!(
        session.debug_records(),
        &[b"Hello from original Switch homebrew!\n".to_vec()]
    );
    assert!(matches!(
        session.run(1),
        Err(SessionError::InvalidState(SessionState::Exited))
    ));
}

#[test]
fn execution_budget_yields_and_resumes_without_repeating_output() {
    let mut session = Session::from_nro(HELLO).unwrap();
    assert_eq!(session.run(3).unwrap(), RunOutcome::BudgetExhausted);
    assert_eq!(session.state(), SessionState::Paused);
    assert!(session.debug_records().is_empty());
    assert_eq!(session.run(1).unwrap(), RunOutcome::BudgetExhausted);
    assert_eq!(session.debug_records().len(), 1);
    assert_eq!(session.run(1).unwrap(), RunOutcome::Exited);
    assert_eq!(session.debug_records().len(), 1);
}

#[test]
fn unsupported_service_faults_the_session_instead_of_faking_success() {
    let mut bytes = HELLO.to_vec();
    bytes[0x80..0x84].copy_from_slice(&0xd4000841_u32.to_le_bytes()); // svc #0x42
    let mut session = Session::from_nro(&bytes).unwrap();
    assert!(matches!(
        session.run(100),
        Err(SessionError::Kernel(
            KernelError::UnsupportedSupervisorCall(0x42)
        ))
    ));
    assert_eq!(session.state(), SessionState::Faulted);
    assert_eq!(session.cpu().pc, LOAD_BASE + 0x84);
    assert!(session.debug_records().is_empty());
}

#[test]
fn original_guest_submits_dma_and_checks_dependent_copies_before_cleanup() {
    let mut session =
        Session::from_nro(include_bytes!("../../../examples/homebrew/gpu-dma.nro")).unwrap();
    assert_eq!(session.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(
        session.debug_records(),
        &[b"Guest DMA copied 16 bytes; dependent copies verified.\n".to_vec()]
    );
    for offset in [0x2010, 0x2020, 0x2030] {
        assert_eq!(session.memory().read_u64(LOAD_BASE + offset).unwrap(), 42);
    }
    assert_eq!(
        session
            .kernel()
            .ipc_records()
            .iter()
            .map(|r| r.command_id)
            .collect::<Vec<_>>(),
        [0, 1, 0, 1]
    );
    assert_eq!(session.kernel().live_handle_count(), 0);
}
