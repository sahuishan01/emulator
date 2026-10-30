use emulator_kernel::KernelError;
use emulator_session::{
    RunOutcome, Session, SessionError, SessionState, CONFIG_BASE, LOAD_BASE, STACK_BASE, STACK_SIZE,
};

const HELLO: &[u8] = include_bytes!("../../../examples/homebrew/hello.nro");

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
