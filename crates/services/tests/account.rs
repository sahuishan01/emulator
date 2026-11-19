//! `acc:u0` list commands over C (pointer receive) buffers.
mod common;
use common::Rig;
use emulator_memory::Permissions;
use emulator_services::acc::{AccountService, DEFAULT_USER_ID};

const LIST: u64 = 0x30000;

#[test]
fn user_lists_fill_the_pointer_buffer_with_the_single_user() {
    let mut rig = Rig::new("acc:u0", Box::new(AccountService));
    rig.m
        .map(LIST, vec![0xee; 0x1000], Permissions::READ_WRITE)
        .unwrap();
    for command in [2, 3] {
        rig.call(command, &[], &[], Some((LIST, 8 * 16)), &[]);
        assert_eq!(rig.result(), 0);
        let mut uids = vec![0u8; 8 * 16];
        rig.m.read(LIST, &mut uids).unwrap();
        assert_eq!(&uids[..16], &DEFAULT_USER_ID);
        assert!(
            uids[16..].iter().all(|&b| b == 0),
            "unused slots hold the invalid uid"
        );
        assert_eq!(
            rig.m.read_u8(LIST + 8 * 16).unwrap(),
            0xee,
            "nothing past the buffer"
        );
    }
    // No stored open-context users.
    rig.m.write(LIST, &[0xee; 16]).unwrap();
    rig.call(60, &[], &[], Some((LIST, 16)), &[]);
    assert_eq!((rig.result(), rig.m.read_u64(LIST).unwrap()), (0, 0));
}

#[test]
fn baas_manager_exists_only_for_the_registered_user() {
    let mut rig = Rig::new("acc:u0", Box::new(AccountService));
    rig.call(101, &[], &[], None, &DEFAULT_USER_ID);
    assert_eq!(rig.handle_result(), 0);
    let manager = rig.moved();
    // No network service account: its commands fail explicitly.
    for command in [0, 1, 2] {
        assert!(
            rig.try_call_on(manager, command, &[], &[], &[]).is_err(),
            "cmd {command}"
        );
    }
    let mut stranger = DEFAULT_USER_ID;
    stranger[15] = 7;
    rig.call(101, &[], &[], None, &stranger);
    assert_eq!(
        rig.handle_result(),
        emulator_services::acc::RESULT_ACCOUNT_USER_NOT_FOUND
    );
    assert!(rig.reply_layout().0.is_empty());
    assert!(
        rig.try_call(101, &[], &[], None, &[0; 8]).is_err(),
        "uid payload required"
    );
}
