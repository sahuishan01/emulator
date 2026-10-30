use emulator_kernel::THREAD_TLS_ARENA;
use emulator_session::{RunOutcome, Session, SessionState, LOAD_BASE};

const THREADS: &[u8] = include_bytes!("../../../examples/homebrew/threads.nro");
const MESSAGE: &str = "Guest threads woke events, joined and timed out.\n";

#[test]
fn owned_two_thread_guest_signals_joins_times_out_and_preserves_context() {
    let mut session = Session::from_nro(THREADS).unwrap();
    assert_eq!(session.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(session.debug_records(), &[MESSAGE.as_bytes().to_vec()]);
    assert_eq!(session.memory().read_u64(LOAD_BASE + 0x2008).unwrap(), 42);
    assert_eq!(
        session.memory().read_u64(LOAD_BASE + 0x2010).unwrap(),
        LOAD_BASE + 0x2000
    );
    assert_eq!(
        session.memory().read_u64(LOAD_BASE + 0x2018).unwrap(),
        THREAD_TLS_ARENA + 0x1200
    );
    assert_eq!(session.memory().read_u64(LOAD_BASE + 0x2020).unwrap(), 42);
    assert_eq!(
        session.memory().read_u64(LOAD_BASE + 0x2028).unwrap(),
        LOAD_BASE + 0x2000
    );
    assert_eq!(
        session.memory().read_u64(THREAD_TLS_ARENA + 0x280).unwrap(),
        0x1111
    );
    assert_eq!(
        session
            .memory()
            .read_u64(THREAD_TLS_ARENA + 0x1280)
            .unwrap(),
        0x2222
    );
    let stats = session.kernel().scheduler_stats();
    assert_eq!(stats.threads_created, 2);
    assert_eq!(stats.blocking_waits, 3);
    assert_eq!(stats.timeouts, 1);
    assert!(stats.context_switches >= 4);
    assert!(stats.virtual_time_ns >= 1600);
    assert_eq!(session.kernel().live_handle_count(), 0);
}

#[test]
fn execution_budgets_do_not_change_guest_thread_order_or_timeout_results() {
    let mut baseline = Session::from_nro(THREADS).unwrap();
    baseline.run(10000).unwrap();
    for budget in [1, 7, 63, 64, 65] {
        let mut session = Session::from_nro(THREADS).unwrap();
        let mut finished = false;
        for _ in 0..2000 {
            match session.run(budget).unwrap() {
                RunOutcome::Exited => {
                    finished = true;
                    break;
                }
                RunOutcome::BudgetExhausted => (),
                RunOutcome::Blocked => panic!("owned fixture unexpectedly deadlocked"),
            }
        }
        assert!(finished);
        assert_eq!(session.instructions(), baseline.instructions());
        assert_eq!(session.debug_records(), baseline.debug_records());
        assert_eq!(
            session.kernel().scheduler_stats(),
            baseline.kernel().scheduler_stats()
        );
        assert_eq!(session.cpu(), baseline.cpu());
    }
}

#[test]
fn infinite_guest_wait_returns_blocked_without_a_host_hang_or_false_exit() {
    // Independently assembled MOVZ W2,#1 / MOVZ X3,#0 sequence in events.S.
    // Change its zero timeout to MOVN X3,#0 (infinite), keeping the owned fixture.
    let mut bytes = include_bytes!("../../../examples/homebrew/events.nro").to_vec();
    let at = bytes
        .windows(4)
        .position(|b| b == 0xd2800003_u32.to_le_bytes())
        .unwrap();
    bytes[at..at + 4].copy_from_slice(&0x92800003_u32.to_le_bytes());
    let mut session = Session::from_nro(&bytes).unwrap();
    assert_eq!(session.run(100).unwrap(), RunOutcome::Blocked);
    assert_eq!(session.state(), SessionState::Blocked);
    let count = session.instructions();
    let stats = session.kernel().scheduler_stats();
    assert_eq!(session.run(100).unwrap(), RunOutcome::Blocked);
    assert_eq!(session.instructions(), count);
    assert_eq!(session.kernel().scheduler_stats(), stats);
    assert!(session.debug_records().is_empty());
    assert!(session.kernel().live_handle_count() > 0);
}
