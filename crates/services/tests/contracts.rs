use emulator_ipc::{CommandType, Request};
use emulator_kernel::service::{Service, ServiceError};
use emulator_services::{DiagnosticCounter, ServiceManager};

fn request(command: u32, data: &[u8], pid: Option<u64>) -> Request {
    Request {
        kind: CommandType::Request,
        tipc: false,
        command_id: command,
        pid,
        data: data.to_vec(),
        send_buffers: Vec::new(),
        receive_buffers: Vec::new(),
        pointer_send: Vec::new(),
        pointer_receive: Vec::new(),
        domain: None,
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

    let mut user_clock = timesys
        .dispatch(&request(0, &[], None))
        .unwrap()
        .moved_objects
        .remove(0);
    let time_reply = user_clock.dispatch(&request(0, &[], None)).unwrap();
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

    // GetDisplayResolution(u64 display_id) -> u64 width, u64 height
    let res_reply = display.dispatch(&request(1102, &[0; 8], None)).unwrap();
    let width = u64::from_le_bytes(res_reply.data[0..8].try_into().unwrap());
    let height = u64::from_le_bytes(res_reply.data[8..16].try_into().unwrap());
    assert_eq!((width, height), (1280, 720));
    assert!(display.dispatch(&request(1102, &[], None)).is_err());
    let mut name = [0u8; 0x40];
    name[..7].copy_from_slice(b"Default");
    assert_eq!(display.dispatch(&request(1010, &name, None)).unwrap().data, 0u64.to_le_bytes());
    name[..7].copy_from_slice(b"Externa");
    assert!(display.dispatch(&request(1010, &name, None)).is_err(), "only the Default display exists");
    assert_eq!(display.dispatch(&request(1011, &[], None)).unwrap().data, 0u64.to_le_bytes());
    // Sub-services are distinct interfaces that fail unknown commands explicitly.
    let mut system = display.dispatch(&request(101, &[], None)).unwrap().moved_objects.remove(0);
    assert_eq!(system.name(), "ISystemDisplayService");
    assert!(system.dispatch(&request(1200, &[], None)).is_err());
    let manager = display.dispatch(&request(102, &[], None)).unwrap().moved_objects.remove(0);
    assert_eq!(manager.name(), "IManagerDisplayService");

    // Relay service (IHOSBinderDriver)
    let mut relay_reply = display.dispatch(&request(100, &[], None)).unwrap();
    assert_eq!(relay_reply.result, 0);
    assert_eq!(relay_reply.moved_objects.len(), 1);
    let mut binder = relay_reply.moved_objects.remove(0);
    assert_eq!(binder.name(), "IHOSBinderDriver");
    // Parcel transactions need their A/B buffers (see vi_layer / binder tests).
    assert!(binder.dispatch(&request(0, &[1, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0], None)).is_err());

    // DisplayService layer and vsync commands
    let vsync = display.dispatch(&request(5202, &[0; 8], None)).unwrap();
    assert!(vsync.moved_objects[0].as_handle().is_some());
    assert!(display.dispatch(&request(2201, &[], None)).is_err(), "unimplemented commands fail");

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

    let mut user_clock_reply = timesys.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(user_clock_reply.result, 0);
    let mut user_clock = user_clock_reply.moved_objects.remove(0);
    let cur_time = user_clock.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(u64::from_le_bytes(cur_time.data.try_into().unwrap()), 1767225600);

    let mut steady_clock_reply = timesys.dispatch(&request(2, &[], None)).unwrap();
    let mut steady_clock = steady_clock_reply.moved_objects.remove(0);
    let pt = steady_clock.dispatch(&request(0, &[], None)).unwrap();
    assert_eq!(pt.data.len(), 0x18);

    // Local clock (4) is an ISystemClock too; unknown commands fail explicitly.
    let mut local = timesys.dispatch(&request(4, &[], None)).unwrap().moved_objects.remove(0);
    assert_eq!(local.dispatch(&request(0, &[], None)).unwrap().result, 0);
    assert_eq!(timesys.dispatch(&request(100, &[], None)).unwrap().data, vec![0]);
    assert!(timesys.dispatch(&request(6, &[], None)).is_err());

    let mut tz_reply = timesys.dispatch(&request(3, &[], None)).unwrap();
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
    // No payload: the reply is exactly one interface object.
    assert!(sc_reply.data.is_empty());
    assert_eq!(sc_reply.moved_objects.len(), 1);
    let mut sc = sc_reply.moved_objects.remove(0);
    let ticks = sc.dispatch(&request(90, &[], None)).unwrap();
    assert_eq!(ticks.data, 0_u64.to_le_bytes());
    let tick_event = sc.dispatch(&request(91, &[], None)).unwrap();
    assert!(tick_event.moved_objects[0].as_handle().is_some());
    assert!(sc.dispatch(&request(91, &[0; 4], None)).is_err());

    assert_eq!(sc.dispatch(&request(0, &[], None)).unwrap().result, 0);
    assert_eq!(sc.dispatch(&request(10, &[], None)).unwrap().result, 0);
    let layer_reply = sc.dispatch(&request(40, &[], None)).unwrap();
    assert_eq!(u64::from_le_bytes(layer_reply.data.try_into().unwrap()), 1);
    let separable = sc.dispatch(&request(44, &[], None)).unwrap();
    assert_eq!(separable.data.len(), 16);
    assert_eq!(u64::from_le_bytes(separable.data[..8].try_into().unwrap()), 1);
    assert_eq!(u64::from_le_bytes(separable.data[8..].try_into().unwrap()), 2);
    // IsSystemBufferSharingEnabled is not implemented and must fail explicitly.
    assert!(sc.dispatch(&request(41, &[], None)).is_err());

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

#[test]
fn vi_service_and_hos_binder_buffer_queue_lifecycle() {
    let mut sm = ServiceManager::default();
    sm.dispatch(&request(0, &[0; 8], Some(1))).unwrap();

    let mut vi_reply = sm.dispatch(&request(1, b"vi:m\0\0\0\0", None)).unwrap();
    let mut vi = vi_reply.moved_objects.remove(0);

    // GetDisplayService (0) -> returns DisplayService
    let mut disp_reply = vi.dispatch(&request(0, &[], None)).unwrap();
    let mut display_service = disp_reply.moved_objects.remove(0);

    // GetRelayService (100) -> returns HosBinderDriver
    let mut relay_reply = display_service.dispatch(&request(100, &[], None)).unwrap();
    let mut binder = relay_reply.moved_objects.remove(0);

    // Parcels travel in A/B buffers, so a context-free dispatch is refused
    // (the protocol itself is covered by emulator_services::binder tests).
    assert!(matches!(
        binder.dispatch(&request(0, &[1, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0], None)),
        Err(ServiceError::MissingMemoryContext)
    ));
    // AdjustRefcount(id, delta, type) and GetNativeHandle(id, type).
    assert_eq!(binder.dispatch(&request(1, &[0; 12], None)).unwrap().result, 0);
    let handle = binder.dispatch(&request(2, &[1, 0, 0, 0, 0x0f, 0, 0, 0], None)).unwrap();
    assert_eq!(handle.moved_objects.len(), 1);
    assert!(binder.dispatch(&request(2, &[0; 4], None)).is_err());
}

#[test]
fn manager_accepts_tipc_registration_without_a_pid_placeholder() {
    let mut sm = ServiceManager::default();
    let mut register = request(0, &[], Some(1));
    register.tipc = true;
    // A CMIF-sized placeholder is not valid TIPC, and TIPC still needs the PID.
    let mut padded = request(0, &[0; 8], Some(1));
    padded.tipc = true;
    assert!(matches!(
        sm.dispatch(&padded),
        Err(ServiceError::InvalidPayload { expected: 0, actual: 8 })
    ));
    let mut anonymous = request(0, &[], None);
    anonymous.tipc = true;
    assert!(matches!(
        sm.dispatch(&anonymous),
        Err(ServiceError::MissingSenderIdentity)
    ));
    assert_eq!(sm.dispatch(&register).unwrap().result, 0);
    let mut get = request(1, b"emu:test", None);
    get.tipc = true;
    assert_eq!(sm.dispatch(&get).unwrap().moved_objects.len(), 1);
}

#[test]
fn hid_server_command_table_round_trips_npad_configuration() {
    let mut sm = ServiceManager::default();
    sm.dispatch(&request(0, &[0; 8], Some(1))).unwrap();
    let mut hid = sm
        .dispatch(&request(1, b"hid\0\0\0\0\0", None))
        .unwrap()
        .moved_objects
        .remove(0);
    let mut revision = 3u32.to_le_bytes().to_vec();
    revision.extend_from_slice(&[0; 12]);
    assert_eq!(hid.dispatch(&request(109, &revision, None)).unwrap().result, 0);

    hid.dispatch(&request(100, &0x3u32.to_le_bytes(), None)).unwrap();
    assert_eq!(hid.dispatch(&request(101, &[], None)).unwrap().data, 0x3u32.to_le_bytes());

    hid.dispatch(&request(120, &[0, 0, 0, 0, 0, 0, 0, 0], None)).unwrap();
    assert_eq!(hid.dispatch(&request(121, &[], None)).unwrap().data, 0u64.to_le_bytes());

    hid.dispatch(&request(128, &[2, 0, 0, 0], None)).unwrap();
    assert_eq!(hid.dispatch(&request(129, &[], None)).unwrap().data, 2u64.to_le_bytes());

    hid.dispatch(&request(1000, &[1, 0, 0, 0], None)).unwrap();
    assert_eq!(hid.dispatch(&request(1001, &[], None)).unwrap().data, 1u64.to_le_bytes());

    let style_event = hid.dispatch(&request(106, &[0; 24], None)).unwrap();
    assert_eq!(style_event.moved_objects.len(), 1);

    let info = hid.dispatch(&request(200, &0x0100_0000u32.to_le_bytes(), None)).unwrap();
    assert_eq!(info.data, [1, 0, 0, 0, 2, 0, 0, 0]);
    let mut devices = hid.dispatch(&request(203, &[], None)).unwrap().moved_objects.remove(0);
    assert_eq!(devices.dispatch(&request(0, &[0; 4], None)).unwrap().result, 0);
    assert!(devices.dispatch(&request(1, &[], None)).is_err());

    hid.dispatch(&request(204, &[0], None)).unwrap();
    assert_eq!(hid.dispatch(&request(205, &[], None)).unwrap().data, [0]);

    // Setters need their payload; unknown commands fail explicitly.
    assert!(hid.dispatch(&request(100, &[], None)).is_err());
    assert!(hid.dispatch(&request(999, &[], None)).is_err());
}

#[test]
fn audio_device_reports_stereo_unit_volume_and_rejects_out_of_range_volume() {
    use emulator_kernel::service::Service;
    let mut device = emulator_services::aud::AudioDevice::default();
    let mut request = |id: u32, data: Vec<u8>| device.dispatch(&request(id, &data, None));
    assert_eq!(request(5, vec![]).unwrap().data, 2u32.to_le_bytes());
    assert_eq!(request(2, vec![]).unwrap().data, 1.0f32.to_le_bytes());
    assert_eq!(request(1, 0.25f32.to_le_bytes().to_vec()).unwrap().result, 0);
    assert_eq!(request(8, vec![]).unwrap().data, 0.25f32.to_le_bytes());
    assert_ne!(request(7, 1.5f32.to_le_bytes().to_vec()).unwrap().result, 0);
    assert_eq!(request(2, vec![]).unwrap().data, 0.25f32.to_le_bytes());
    assert!(request(1, vec![]).is_err());
    assert_eq!(request(4, vec![]).unwrap().moved_objects.len(), 1);
    assert!(request(0, vec![]).is_err());
}
