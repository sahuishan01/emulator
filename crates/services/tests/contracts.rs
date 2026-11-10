use emulator_ipc::{CommandType, Request};
use emulator_kernel::service::{Service, ServiceError};
use emulator_services::{DiagnosticCounter, ServiceManager};

fn request(command: u32, data: &[u8], pid: Option<u64>) -> Request {
    Request {
        kind: CommandType::Request,
        command_id: command,
        pid,
        data: data.to_vec(),
        send_buffers: Vec::new(),
        receive_buffers: Vec::new(),
        copy_handles: Vec::new(),
    }
}

#[test]
fn manager_requires_initialization_identity_and_a_known_service() {
    let mut sm = ServiceManager::default();
    assert!(matches!(
        sm.dispatch(&request(1, b"emu:test", None)),
        Err(ServiceError::NotInitialized)
    ));
    assert!(matches!(
        sm.dispatch(&request(0, &[0; 8], None)),
        Err(ServiceError::MissingSenderIdentity)
    ));
    sm.dispatch(&request(0, &[0; 8], Some(1))).unwrap();
    assert!(matches!(
        sm.dispatch(&request(0, &[0; 8], Some(1))),
        Err(ServiceError::AlreadyInitialized)
    ));
    assert!(matches!(
        sm.dispatch(&request(1, b"unknown!", None)),
        Err(ServiceError::UnsupportedService(_))
    ));
    let mut first = sm
        .dispatch(&request(1, b"emu:test", None))
        .unwrap()
        .moved_objects
        .remove(0);
    let mut second = sm
        .dispatch(&request(1, b"emu:test", None))
        .unwrap()
        .moved_objects
        .remove(0);
    assert_eq!(
        first
            .dispatch(&request(0, &42_u64.to_le_bytes(), None))
            .unwrap()
            .data,
        42_u64.to_le_bytes()
    );
    assert_eq!(
        second.dispatch(&request(1, &[], None)).unwrap().data,
        0_u64.to_le_bytes()
    );
}

#[test]
fn counter_overflow_and_invalid_payloads_preserve_previous_value() {
    let mut counter = DiagnosticCounter::default();
    counter
        .dispatch(&request(0, &u64::MAX.to_le_bytes(), None))
        .unwrap();
    assert!(matches!(
        counter.dispatch(&request(0, &1_u64.to_le_bytes(), None)),
        Err(ServiceError::ArithmeticOverflow)
    ));
    assert!(matches!(
        counter.dispatch(&request(0, &[], None)),
        Err(ServiceError::InvalidPayload { .. })
    ));
    assert_eq!(
        counter.dispatch(&request(1, &[], None)).unwrap().data,
        u64::MAX.to_le_bytes()
    );
    assert!(matches!(
        counter.dispatch(&request(99, &[], None)),
        Err(ServiceError::UnsupportedCommand { .. })
    ));
}

#[test]
fn test_system_settings_and_time_services() {
    let mut sm = ServiceManager::default();
    sm.dispatch(&request(0, &[0; 8], Some(1))).unwrap();

    let mut setsys = sm
        .dispatch(&request(1, b"set:sys\0", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let lang_reply = setsys.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(&lang_reply.data[..5], b"en-US");

    let region_reply = setsys.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(region_reply.data.try_into().unwrap()), 1);

    let mut timesys = sm
        .dispatch(&request(1, b"time:s\0\0", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let time_reply = timesys.dispatch(&request(0, &[], None)).unwrap();
    let seconds = u64::from_le_bytes(time_reply.data.try_into().unwrap());
    assert!(seconds >= 1700000000);
}

#[test]
fn test_fsp_srv_filesystem_service() {
    let mut sm = ServiceManager::default();
    sm.dispatch(&request(0, &[0; 8], Some(1))).unwrap();

    let mut fspsrv = sm
        .dispatch(&request(1, b"fsp-srv\0", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let init_reply = fspsrv.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(init_reply.result, 0);

    let mut romfs_reply = fspsrv.dispatch(&request(2, &[], None)).unwrap();
    assert_eq!(romfs_reply.result, 0);
    assert_eq!(romfs_reply.moved_objects.len(), 1);

    let mut filesystem = romfs_reply.moved_objects.remove(0);
    let open_reply = filesystem.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(open_reply.result, 0);
}

#[test]
fn test_vi_and_applet_services() {
    let mut sm = ServiceManager::default();
    sm.dispatch(&request(0, &[0; 8], Some(1))).unwrap();

    let mut vi = sm
        .dispatch(&request(1, b"vi:m\0\0\0\0", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let mut display_reply = vi.dispatch(&request(0, &[], None)).unwrap();
    let mut display = display_reply.moved_objects.remove(0);

    let res_reply = display.dispatch(&request(1020, &[], None)).unwrap();
    let width = u32::from_le_bytes(res_reply.data[0..4].try_into().unwrap());
    let height = u32::from_le_bytes(res_reply.data[4..8].try_into().unwrap());
    assert_eq!((width, height), (1280, 720));

    let mut applet = sm
        .dispatch(&request(1, b"appletAE", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let mut applet_reply = applet.dispatch(&request(0, &[], None)).unwrap();
    let mut session = applet_reply.moved_objects.remove(0);

    let mode_reply = session.dispatch(&request(21, &[], None)).unwrap();
    assert_eq!(mode_reply.data[0], 0); // Handheld mode

    let mut hid = sm
        .dispatch(&request(1, b"hid\0\0\0\0\0", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let applet_res_reply = hid.dispatch(&request(0, &[1, 0, 0, 0, 0, 0, 0, 0], None)).unwrap();
    assert_eq!(applet_res_reply.result, 0);
    assert_eq!(applet_res_reply.moved_objects.len(), 1);

    let npad_reply = hid.dispatch(&request(100, &[1, 0, 0, 0, 0, 0, 0, 0], None)).unwrap();
    assert_eq!(npad_reply.result, 0);
}
