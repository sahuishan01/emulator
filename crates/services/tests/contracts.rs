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

    // Relay service (IHOSBinderDriver)
    let mut relay_reply = display.dispatch(&request(100, &[], None)).unwrap();
    assert_eq!(relay_reply.result, 0);
    assert_eq!(relay_reply.moved_objects.len(), 1);
    let mut binder = relay_reply.moved_objects.remove(0);

    // Binder Connect (code 10)
    let connect_data = [1, 0, 0, 0, 10, 0, 0, 0];
    let connect_reply = binder.dispatch(&request(0, &connect_data, None)).unwrap();
    assert_eq!(connect_reply.result, 0);

    // Binder DequeueBuffer (code 3)
    let dequeue_data = [1, 0, 0, 0, 3, 0, 0, 0];
    let dequeue_reply = binder.dispatch(&request(0, &dequeue_data, None)).unwrap();
    assert_eq!(dequeue_reply.result, 0);

    // Binder RequestBuffer (code 1)
    let req_buf_data = [1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    let req_buf_reply = binder.dispatch(&request(0, &req_buf_data, None)).unwrap();
    assert_eq!(req_buf_reply.result, 0);
    assert_eq!(req_buf_reply.data.len(), 28);
    let buf_width = u32::from_le_bytes(req_buf_reply.data[8..12].try_into().unwrap());
    let buf_height = u32::from_le_bytes(req_buf_reply.data[12..16].try_into().unwrap());
    assert_eq!((buf_width, buf_height), (1280, 720));

    // Binder QueueBuffer (code 7)
    let queue_data = [1, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0];
    let queue_reply = binder.dispatch(&request(0, &queue_data, None)).unwrap();
    assert_eq!(queue_reply.result, 0);

    // Binder Query (code 9)
    let query_width = [1, 0, 0, 0, 9, 0, 0, 0, 1, 0, 0, 0];
    let q_w_reply = binder.dispatch(&request(0, &query_width, None)).unwrap();
    assert_eq!(u32::from_le_bytes(q_w_reply.data[4..8].try_into().unwrap()), 1280);

    let query_height = [1, 0, 0, 0, 9, 0, 0, 0, 2, 0, 0, 0];
    let q_h_reply = binder.dispatch(&request(0, &query_height, None)).unwrap();
    assert_eq!(u32::from_le_bytes(q_h_reply.data[4..8].try_into().unwrap()), 720);

    let query_fmt = [1, 0, 0, 0, 9, 0, 0, 0, 3, 0, 0, 0];
    let q_f_reply = binder.dispatch(&request(0, &query_fmt, None)).unwrap();
    assert_eq!(u32::from_le_bytes(q_f_reply.data[4..8].try_into().unwrap()), 1);

    // Binder CancelBuffer (code 8) & Disconnect (code 11)
    let cancel_data = [1, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(binder.dispatch(&request(0, &cancel_data, None)).unwrap().result, 0);
    let disconn_data = [1, 0, 0, 0, 11, 0, 0, 0];
    assert_eq!(binder.dispatch(&request(0, &disconn_data, None)).unwrap().result, 0);

    // DisplayService layer and vsync commands
    assert_eq!(display.dispatch(&request(2201, &[], None)).unwrap().result, 0);
    assert_eq!(display.dispatch(&request(2203, &[], None)).unwrap().result, 0);
    assert_eq!(display.dispatch(&request(8225, &[], None)).unwrap().result, 0);

    // Account Service (acc:u0)
    let mut acc = sm
        .dispatch(&request(1, b"acc:u0\0\0", None))
        .unwrap()
        .moved_objects
        .remove(0);
    let user_count_reply = acc.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(i32::from_le_bytes(user_count_reply.data.try_into().unwrap()), 1);
    let mut open_user_reply = acc.dispatch(&request(3, &[1, 0, 0, 0], None)).unwrap();
    assert_eq!(open_user_reply.moved_objects.len(), 1);
    let mut profile = open_user_reply.moved_objects.remove(0);
    let profile_base = profile.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(&profile_base.data[0x18..0x18 + 6], b"Player");

    // Performance Management Service (apm)
    let mut apm = sm
        .dispatch(&request(1, b"apm\0\0\0\0\0", None))
        .unwrap()
        .moved_objects
        .remove(0);
    let perf_mode_reply = apm.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(perf_mode_reply.data.try_into().unwrap()), 0); // Handheld

    // Audio Renderer Service (audren:u)
    let mut audren = sm
        .dispatch(&request(1, b"audren:u", None))
        .unwrap()
        .moved_objects
        .remove(0);
    let buf_size_reply = audren.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(u64::from_le_bytes(buf_size_reply.data.try_into().unwrap()), 0x40000);

    // Audio Output Service (audout:u)
    let mut audout = sm
        .dispatch(&request(1, b"audout:u", None))
        .unwrap()
        .moved_objects
        .remove(0);
    let list_outs = audout.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(&list_outs.data[..9], b"Headphone");

    // Verify port aliases
    assert!(sm.dispatch(&request(1, b"vi:u\0\0\0\0", None)).is_ok());
    assert!(sm.dispatch(&request(1, b"vi:s\0\0\0\0", None)).is_ok());
    assert!(sm.dispatch(&request(1, b"set\0\0\0\0\0", None)).is_ok());

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

#[test]
fn test_applet_subcontrollers_and_extended_services() {
    let mut sm = ServiceManager::default();
    sm.dispatch(&request(0, &[0; 8], Some(1))).unwrap();

    // Verify newly added service port aliases
    assert!(sm.dispatch(&request(1, b"time:a\0\0", None)).is_ok());
    assert!(sm.dispatch(&request(1, b"nvdrv:t\0", None)).is_ok());
    assert!(sm.dispatch(&request(1, b"fsp-pr\0\0", None)).is_ok());
    assert!(sm.dispatch(&request(1, b"apm:sys\0", None)).is_ok());

    // Test set:sys extended commands
    let mut setsys = sm
        .dispatch(&request(1, b"set:sys\0", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let sys_lang = setsys.dispatch(&request(2, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(sys_lang.data.try_into().unwrap()), 1);

    let album = setsys.dispatch(&request(5, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(album.data.try_into().unwrap()), 1);

    let model = setsys.dispatch(&request(8, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(model.data.try_into().unwrap()), 1);

    let nickname = setsys.dispatch(&request(11, &[], None)).unwrap();
    assert_eq!(&nickname.data[..6], b"Switch");

    let theme = setsys.dispatch(&request(3, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(theme.data.try_into().unwrap()), 0); // BasicBlack

    let notif = setsys.dispatch(&request(13, &[], None)).unwrap();
    assert_eq!(f32::from_le_bytes(notif.data.try_into().unwrap()), 1.0);

    let fw = setsys.dispatch(&request(14, &[], None)).unwrap();
    assert_eq!(fw.data[0], 18);
    assert_eq!(fw.data[1], 1);
    assert_eq!(&fw.data[0x28..0x28 + 6], b"18.1.0");

    // Test time:s extended commands
    let mut timesys = sm
        .dispatch(&request(1, b"time:s\0\0", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let mut user_clock_reply = timesys.dispatch(&request(3, &[], None)).unwrap();
    assert_eq!(user_clock_reply.result, 0);
    let mut user_clock = user_clock_reply.moved_objects.remove(0);
    let cur_time = user_clock.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(u64::from_le_bytes(cur_time.data.try_into().unwrap()), 1767225600);

    let mut steady_clock_reply = timesys.dispatch(&request(5, &[], None)).unwrap();
    let mut steady_clock = steady_clock_reply.moved_objects.remove(0);
    let pt = steady_clock.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(pt.data.len(), 0x18);

    let mut tz_reply = timesys.dispatch(&request(6, &[], None)).unwrap();
    let mut tz = tz_reply.moved_objects.remove(0);
    let loc = tz.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(&loc.data[..3], b"UTC");

    // Test appletOE / appletAE full controller hierarchy
    let mut applet = sm
        .dispatch(&request(1, b"appletOE", None))
        .unwrap()
        .moved_objects
        .remove(0);

    let mut proxy_reply = applet.dispatch(&request(200, &[], None)).unwrap();
    assert_eq!(proxy_reply.result, 0);
    let mut proxy = proxy_reply.moved_objects.remove(0);

    // 1. GetCommonStateGetter (command 0)
    let mut csg_reply = proxy.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(csg_reply.result, 0);
    let mut csg = csg_reply.moved_objects.remove(0);

    let event_reply = csg.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(event_reply.result, 0);
    assert_eq!(event_reply.moved_objects.len(), 1);

    let msg_reply = csg.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(msg_reply.data.try_into().unwrap()), 0x0F);

    let op_mode = csg.dispatch(&request(5, &[], None)).unwrap();
    assert_eq!(op_mode.data[0], 0);

    let perf_mode = csg.dispatch(&request(6, &[], None)).unwrap();
    assert_eq!(u32::from_le_bytes(perf_mode.data.try_into().unwrap()), 0);

    let focus = csg.dispatch(&request(9, &[], None)).unwrap();
    assert_eq!(focus.data[0], 1);

    // 2. GetSelfController (command 1)
    let mut sc_reply = proxy.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(sc_reply.result, 0);
    assert_eq!(u64::from_le_bytes(sc_reply.data.try_into().unwrap()), 1);
    let mut sc = sc_reply.moved_objects.remove(0);

    assert_eq!(sc.dispatch(&request(0, &[], None)).unwrap().result, 0);
    assert_eq!(sc.dispatch(&request(10, &[], None)).unwrap().result, 0);
    let layer_reply = sc.dispatch(&request(40, &[], None)).unwrap();
    assert_eq!(u64::from_le_bytes(layer_reply.data.try_into().unwrap()), 1);

    // 3. GetWindowController (command 2)
    let mut wc_reply = proxy.dispatch(&request(2, &[], None)).unwrap();
    let mut wc = wc_reply.moved_objects.remove(0);
    let uid_reply = wc.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(u64::from_le_bytes(uid_reply.data.try_into().unwrap()), 1);

    // 4. GetAudioController (command 3)
    let mut ac_reply = proxy.dispatch(&request(3, &[], None)).unwrap();
    let mut ac = ac_reply.moved_objects.remove(0);
    let vol_reply = ac.dispatch(&request(1, &[], None)).unwrap();
    assert_eq!(f32::from_le_bytes(vol_reply.data.try_into().unwrap()), 1.0);

    // 5. GetApplicationFunctions (command 20)
    let mut af_reply = proxy.dispatch(&request(20, &[], None)).unwrap();
    let mut af = af_reply.moved_objects.remove(0);

    let lang_reply = af.dispatch(&request(21, &[], None)).unwrap();
    assert_eq!(&lang_reply.data[..5], b"en-US");

    let ver_reply = af.dispatch(&request(23, &[], None)).unwrap();
    assert_eq!(&ver_reply.data[..5], b"1.0.0");

    let running_reply = af.dispatch(&request(40, &[], None)).unwrap();
    assert_eq!(running_reply.data[0], 1);

    let pseudo_reply = af.dispatch(&request(50, &[], None)).unwrap();
    assert_eq!(pseudo_reply.data.len(), 16);
}
