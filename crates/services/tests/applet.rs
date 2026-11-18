//! `IApplicationFunctions` launch parameters, applet storages and the
//! `ICommonStateGetter` message queue, driven through real CMIF requests.
mod common;
use common::Rig;
use emulator_memory::Permissions;
use emulator_services::acc::DEFAULT_USER_ID;
use emulator_services::applet::{
    ApplicationProxy, LAUNCH_PARAMETER_PRESELECTED_USER, LAUNCH_PARAMETER_USER_CHANNEL,
    MESSAGE_FOCUS_STATE_CHANGED, PRESELECTED_USER_MAGIC, PRESELECTED_USER_SIZE,
    RESULT_AM_INVALID_OFFSET, RESULT_AM_NO_DATA_IN_CHANNEL, RESULT_AM_NO_MESSAGES,
};

const BUFFER: u64 = 0x30000;

fn proxy() -> Rig {
    let mut rig = Rig::new("appletprx", Box::new(ApplicationProxy::default()));
    rig.m.map(BUFFER, vec![0xee; 0x1000], Permissions::READ_WRITE).unwrap();
    rig
}

/// GetApplicationFunctions (proxy command 20).
fn functions(rig: &mut Rig) -> u32 {
    rig.call(20, &[], &[], None, &[]);
    assert_eq!(rig.handle_result(), 0);
    rig.moved()
}

fn read(rig: &mut Rig, accessor: u32, offset: i64, length: u64) -> (u32, Vec<u8>) {
    rig.m.write(BUFFER, &vec![0xee; length as usize]).unwrap();
    rig.call_on(accessor, 11, &[], &[(BUFFER, length)], &offset.to_le_bytes());
    let mut bytes = vec![0u8; length as usize];
    rig.m.read(BUFFER, &mut bytes).unwrap();
    (rig.result(), bytes)
}

#[test]
fn preselected_user_is_popped_once_as_a_readable_storage() {
    let mut rig = proxy();
    let functions = functions(&mut rig);

    rig.call_on(functions, 1, &[], &[], &LAUNCH_PARAMETER_PRESELECTED_USER.to_le_bytes());
    assert_eq!(rig.handle_result(), 0);
    let storage = rig.moved();
    rig.call_on(storage, 0, &[], &[], &[]);
    let accessor = rig.moved();

    rig.call_on(accessor, 0, &[], &[], &[]);
    assert_eq!(rig.result(), 0);
    assert_eq!(rig.out(8), (PRESELECTED_USER_SIZE as i64).to_le_bytes());

    let (result, bytes) = read(&mut rig, accessor, 0, PRESELECTED_USER_SIZE as u64);
    assert_eq!(result, 0);
    assert_eq!(bytes[..4], PRESELECTED_USER_MAGIC.to_le_bytes());
    assert_eq!(bytes[4], 1, "is_account_selected");
    assert_eq!(bytes[8..24], DEFAULT_USER_ID);
    assert!(bytes[24..].iter().all(|&b| b == 0));

    // The channel is empty after one pop; the user channel never had data.
    rig.call_on(functions, 1, &[], &[], &LAUNCH_PARAMETER_PRESELECTED_USER.to_le_bytes());
    assert_eq!(rig.handle_result(), RESULT_AM_NO_DATA_IN_CHANNEL);
    assert!(rig.reply_layout().0.is_empty(), "no storage on failure");
    rig.call_on(functions, 1, &[], &[], &LAUNCH_PARAMETER_USER_CHANNEL.to_le_bytes());
    assert_eq!(rig.handle_result(), RESULT_AM_NO_DATA_IN_CHANNEL);
    // Unknown kinds and malformed payloads are rejected, not answered.
    assert!(rig.try_call_on(functions, 1, &[], &[], &7u32.to_le_bytes()).is_err());
    assert!(rig.try_call_on(functions, 1, &[], &[], &[]).is_err());
}

#[test]
fn storage_access_is_bounded_and_writes_outlive_the_accessor() {
    let mut rig = proxy();
    let functions = functions(&mut rig);
    rig.call_on(functions, 1, &[], &[], &LAUNCH_PARAMETER_PRESELECTED_USER.to_le_bytes());
    let storage = rig.moved();
    rig.call_on(storage, 0, &[], &[], &[]);
    let accessor = rig.moved();

    // Out of range reads and writes fail without touching guest memory.
    for (offset, length) in [(0x80, 0x10), (0x89, 0), (-1, 1)] {
        let (result, bytes) = read(&mut rig, accessor, offset, length);
        assert_eq!(result, RESULT_AM_INVALID_OFFSET, "read {offset:#x}+{length:#x}");
        assert!(bytes.iter().all(|&b| b == 0xee));
    }
    rig.m.write(BUFFER, &[0x5a; 0x10]).unwrap();
    rig.call_on(accessor, 10, &[(BUFFER, 0x10)], &[], &0x80i64.to_le_bytes());
    assert_eq!(rig.result(), RESULT_AM_INVALID_OFFSET);

    // A tail write lands, and a second accessor on the same storage sees it.
    rig.call_on(accessor, 10, &[(BUFFER, 0x10)], &[], &0x78i64.to_le_bytes());
    assert_eq!(rig.result(), 0);
    rig.call_on(storage, 0, &[], &[], &[]);
    let second = rig.moved();
    let (result, bytes) = read(&mut rig, second, 0x70, 0x18);
    assert_eq!(result, 0);
    assert_eq!(bytes[..8], [0; 8]);
    assert_eq!(bytes[8..], [0x5a; 0x10]);

    // A failed write (bad payload) leaves the storage unchanged.
    rig.m.write(BUFFER, &[0x11; 0x10]).unwrap();
    assert!(rig.try_call_on(accessor, 10, &[(BUFFER, 0x10)], &[], &[0; 4]).is_err());
    let (_, bytes) = read(&mut rig, second, 0x78, 0x10);
    assert_eq!(bytes, [0x5a; 0x10]);

    // Transfer storages do not exist here.
    assert!(rig.try_call_on(storage, 1, &[], &[], &[]).is_err());
}

/// svcWaitSynchronization on one handle with a zero timeout.
fn poll(rig: &mut Rig, handle: u32) -> u64 {
    rig.m.write(BUFFER, &handle.to_le_bytes()).unwrap();
    rig.c.x[1] = BUFFER;
    rig.c.x[2] = 1;
    rig.c.x[3] = 0;
    rig.k.dispatch(emulator_kernel::SVC_WAIT_SYNCHRONIZATION, &mut rig.c, &mut rig.m).unwrap();
    rig.c.x[0]
}

#[test]
fn message_queue_drains_and_clears_its_event() {
    let mut rig = proxy();
    rig.call(0, &[], &[], None, &[]);
    let getter = rig.moved();
    rig.call_on(getter, 0, &[], &[], &[]);
    let event = rig.moved();
    assert_eq!(poll(&mut rig, event), 0, "signaled while a message is queued");

    rig.call_on(getter, 1, &[], &[], &[]);
    assert_eq!((rig.result(), rig.out(4)), (0, MESSAGE_FOCUS_STATE_CHANGED.to_le_bytes().to_vec()));
    rig.call_on(getter, 1, &[], &[], &[]);
    assert_eq!(rig.result(), RESULT_AM_NO_MESSAGES);
    assert_eq!(poll(&mut rig, event), emulator_kernel::RESULT_TIMEOUT, "cleared once drained");

    // A second getter shares the process-wide (now empty) queue.
    rig.call(0, &[], &[], None, &[]);
    let other = rig.moved();
    rig.call_on(other, 1, &[], &[], &[]);
    assert_eq!(rig.result(), RESULT_AM_NO_MESSAGES);
}

#[test]
fn unimplemented_applet_interfaces_fail_explicitly() {
    let mut rig = proxy();
    for (getter, command) in [(4, 0), (11, 0), (1000, 0)] {
        rig.call(getter, &[], &[], None, &[]);
        let interface = rig.moved();
        assert!(rig.try_call_on(interface, command, &[], &[], &[]).is_err(), "proxy {getter} cmd {command}");
    }
    let functions = functions(&mut rig);
    // ExecuteProgram and CreateApplicationAndRequestToStart are not modelled.
    for command in [12, 120] {
        assert!(rig.try_call_on(functions, command, &[], &[], &[]).is_err());
    }
}
