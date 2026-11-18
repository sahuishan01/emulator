//! vi vsync: the display's vsync event is driven by a kernel timer at 60 Hz
//! of guest time, so a thread blocked on it wakes once per refresh.
mod common;
use common::Rig;
use emulator_kernel::{
    ScheduleOutcome, SVC_RESET_SIGNAL, SVC_WAIT_SYNCHRONIZATION,
};
use emulator_memory::Permissions;
use emulator_services::vi::{ViManagerService, VSYNC_PERIOD_NS};

const HANDLES: u64 = 0x5000;

fn vsync_event(rig: &mut Rig, display: u32) -> u32 {
    rig.call_on(display, 5202, &[], &[], &0u64.to_le_bytes());
    assert_eq!(rig.handle_result(), 0);
    rig.moved()
}

/// Block the current thread on `handle` with no timeout and let the
/// scheduler run until it wakes; returns the wait result.
fn wait(rig: &mut Rig, handle: u32) -> u64 {
    rig.m.write_u32(HANDLES, handle).unwrap();
    rig.c.x[1] = HANDLES;
    rig.c.x[2] = 1;
    rig.c.x[3] = u64::MAX; // -1: wait forever
    rig.k.dispatch(SVC_WAIT_SYNCHRONIZATION, &mut rig.c, &mut rig.m).unwrap();
    assert_eq!(rig.k.prepare_execution(&mut rig.c), ScheduleOutcome::Ready);
    rig.c.x[0]
}

fn reset(rig: &mut Rig, handle: u32) {
    rig.c.x[0] = u64::from(handle);
    rig.k.dispatch(SVC_RESET_SIGNAL, &mut rig.c, &mut rig.m).unwrap();
    assert_eq!(rig.c.x[0], 0);
}

#[test]
fn vsync_event_wakes_a_blocked_thread_every_refresh() {
    let mut rig = Rig::new("vi:u", Box::new(ViManagerService::with_graphics(None)));
    rig.m.map(HANDLES, vec![0; 0x1000], Permissions::READ_WRITE).unwrap();
    rig.call(0, &[], &[], None, &[]);
    let display = rig.moved();
    let start = rig.k.scheduler_stats().virtual_time_ns;
    let event = vsync_event(&mut rig, display);
    assert_eq!(rig.k.timers().len(), 1);

    assert_eq!(wait(&mut rig, event), 0);
    assert_eq!(rig.k.scheduler_stats().virtual_time_ns, start + VSYNC_PERIOD_NS);
    reset(&mut rig, event);
    assert_eq!(wait(&mut rig, event), 0);
    assert_eq!(rig.k.scheduler_stats().virtual_time_ns, start + 2 * VSYNC_PERIOD_NS);

    // Asking again returns the same display event without a second timer.
    let again = vsync_event(&mut rig, display);
    assert_eq!(rig.k.timers().len(), 1);
    reset(&mut rig, again);
    assert_eq!(wait(&mut rig, event), 0);
    assert_eq!(rig.k.scheduler_stats().virtual_time_ns, start + 3 * VSYNC_PERIOD_NS);

    // Unknown displays are rejected.
    assert!(rig.try_call_on(display, 5202, &[], &[], &1u64.to_le_bytes()).is_err());
}
