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

#[test]
fn guest_hipc_buffers_grant_dma_input_and_output_and_record_exact_counts() {
    let mut session =
        Session::from_nro(include_bytes!("../../../examples/homebrew/gpu-buffers.nro")).unwrap();
    assert_eq!(session.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(session.instructions(), 327);
    assert_eq!(
        session.debug_records(),
        &[b"Guest HIPC buffers submitted DMA and returned bytes.\n".to_vec()]
    );
    for offset in [0x2010, 0x2020] {
        assert_eq!(session.memory().read_u64(LOAD_BASE + offset).unwrap(), 42);
    }
    let records = session.kernel().ipc_records();
    assert_eq!(records.len(), 4);
    assert_eq!(
        records.iter().map(|r| r.command_id).collect::<Vec<_>>(),
        [0, 1, 2, 1]
    );
    assert_eq!(
        (
            records[2].send_buffers,
            records[2].receive_buffers,
            records[2].buffer_bytes
        ),
        (3, 1, 80)
    );
    assert_eq!(session.kernel().live_handle_count(), 0);
}

#[test]
fn guest_nv_initialization_retains_copied_memory_and_restores_on_final_close() {
    let mut session = Session::from_nro(include_bytes!(
        "../../../examples/homebrew/nv-initialize.nro"
    ))
    .unwrap();
    assert_eq!(session.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(session.instructions(), 317);
    assert_eq!(
        session.debug_records(),
        &[b"Guest NV initialized with copied handles; memory restored.\n".to_vec()]
    );
    assert_eq!(session.memory().read_u64(LOAD_BASE + 0x2000).unwrap(), 42);
    assert_eq!(
        session
            .kernel()
            .ipc_records()
            .iter()
            .map(|r| r.command_id)
            .collect::<Vec<_>>(),
        [0, 1, 3, 2]
    );
    assert_eq!(
        session
            .kernel()
            .ipc_records()
            .iter()
            .map(|r| r.copy_handles)
            .sum::<usize>(),
        2
    );
    assert_eq!(session.kernel().live_handle_count(), 0);
    assert_eq!(session.kernel().live_transfer_memory_count(), 0);
    assert_eq!(session.memory().lent_bytes(), 0);
}

#[test]
fn guest_nvmap_uses_shared_fd_and_buffered_ioctls_then_releases_arena() {
    let mut session =
        Session::from_nro(include_bytes!("../../../examples/homebrew/nvmap.nro")).unwrap();
    assert_eq!(session.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(session.instructions(), 719);
    assert_eq!(
        session.debug_records(),
        &[b"Guest NVMap created, queried and freed; descriptors closed.\n".to_vec()]
    );
    assert_eq!(session.memory().read_u64(LOAD_BASE + 0x3030).unwrap(), 4096);
    assert_eq!(session.memory().read_u64(LOAD_BASE + 0x2000).unwrap(), 42);
    let records = session.kernel().ipc_records();
    assert_eq!(
        records.iter().map(|r| r.command_id).collect::<Vec<_>>(),
        [0, 1, 3, 0, 2, 1, 1, 1, 2]
    );
    assert_eq!(
        (
            records.iter().map(|r| r.send_buffers).sum::<usize>(),
            records.iter().map(|r| r.receive_buffers).sum::<usize>(),
            records.iter().map(|r| r.buffer_bytes).sum::<usize>(),
            records.iter().map(|r| r.copy_handles).sum::<usize>()
        ),
        (4, 3, 98, 2)
    );
    assert_eq!(session.kernel().live_handle_count(), 0);
    assert_eq!(session.kernel().live_transfer_memory_count(), 0);
    assert_eq!(session.memory().lent_bytes(), 0);
}

#[test]
fn guest_nvmap_imports_existing_backing_and_reports_alignment_and_free_address() {
    let mut session =
        Session::from_nro(include_bytes!("../../../examples/homebrew/nvmap-alloc.nro")).unwrap();
    assert_eq!(session.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(session.instructions(), 817);
    assert_eq!(
        session.debug_records(),
        &[b"Guest NVMap imported backing, queried alignment and freed.\n".to_vec()]
    );
    for (offset, value) in [(0x2000, 42), (0x3030, 4096), (0x4000, 77)] {
        assert_eq!(
            session.memory().read_u64(LOAD_BASE + offset).unwrap(),
            value
        );
    }
    assert_eq!(
        session
            .kernel()
            .ipc_records()
            .iter()
            .map(|r| r.command_id)
            .collect::<Vec<_>>(),
        [0, 1, 3, 0, 2, 1, 1, 1, 1, 2]
    );
    let records = session.kernel().ipc_records();
    assert_eq!(
        (
            records.iter().map(|r| r.send_buffers).sum::<usize>(),
            records.iter().map(|r| r.receive_buffers).sum::<usize>(),
            records.iter().map(|r| r.buffer_bytes).sum::<usize>(),
            records.iter().map(|r| r.copy_handles).sum::<usize>()
        ),
        (5, 4, 162, 2)
    );
    assert_eq!(session.kernel().live_handle_count(), 0);
    assert_eq!(session.kernel().live_transfer_memory_count(), 0);
    assert_eq!(session.memory().lent_bytes(), 0);
}

#[test]
fn nv_guest_maps_retained_backing_clones_client_frees_handle_and_unmaps() {
    let mut session =
        Session::from_nro(include_bytes!("../../../examples/homebrew/nv-gpu-map.nro")).unwrap();
    assert_eq!(session.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(session.instructions(), 3172);
    assert_eq!(
        session.debug_records(),
        &[b"Guest NV GPU mappings retained backing and unmapped cleanly.\n".to_vec()]
    );
    for (offset, value) in [(0x2000, 42), (0x3040, 1 << 32), (0x4000, 77)] {
        assert_eq!(
            session.memory().read_u64(LOAD_BASE + offset).unwrap(),
            value
        );
    }
    let records = session.kernel().ipc_records();
    assert_eq!(records.len(), 15);
    assert_eq!(
        records.iter().map(|r| r.command_id).collect::<Vec<_>>(),
        [0, 1, 3, 0, 1, 1, 0, 1, 1, 2, 1, 2, 1, 1, 2]
    );
    assert_eq!(
        (
            records.iter().map(|r| r.send_buffers).sum::<usize>(),
            records.iter().map(|r| r.receive_buffers).sum::<usize>(),
            records.iter().map(|r| r.buffer_bytes).sum::<usize>(),
            records.iter().map(|r| r.copy_handles).sum::<usize>()
        ),
        (9, 6, 372, 2)
    );
    assert_eq!(session.kernel().live_handle_count(), 0);
    assert_eq!(session.kernel().live_transfer_memory_count(), 0);
    assert_eq!(session.memory().lent_bytes(), 0);
}

#[test]
fn guest_nv_batch_executes_rejects_retries_tracks_fences_and_retains_closed_bindings() {
    let mut s = Session::from_nro(include_bytes!(
        "../../../examples/homebrew/nv-channel-batch.nro"
    ))
    .unwrap();
    assert_eq!(s.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(s.instructions(), 7179);
    assert_eq!(
        s.debug_records(),
        &[b"Guest NV batch passed: jobs, fences, rejection and lifetime.\n".to_vec()]
    );
    for (a, v) in [(0x3000, 42), (0x4040, 3), (0x5100, 44), (0x5108, 44)] {
        assert_eq!(s.memory().read_u64(LOAD_BASE + a).unwrap(), v);
    }
    let r = s.kernel().ipc_records();
    assert_eq!(r.len(), 32);
    assert_eq!(
        (
            r.iter().map(|r| r.send_buffers).sum::<usize>(),
            r.iter().map(|r| r.receive_buffers).sum::<usize>(),
            r.iter().map(|r| r.buffer_bytes).sum::<usize>(),
            r.iter().map(|r| r.copy_handles).sum::<usize>()
        ),
        (25, 18, 875, 2)
    );
    assert_eq!(s.kernel().live_handle_count(), 0);
    assert_eq!(s.kernel().live_transfer_memory_count(), 0);
    assert_eq!(s.memory().lent_bytes(), 0);
}

#[test]
fn guest_color_clear_batch_checks_pixels_rejection_fences_and_closed_owners() {
    let mut s = Session::from_nro(include_bytes!(
        "../../../examples/homebrew/nv-color-clear.nro"
    ))
    .unwrap();
    assert_eq!(s.run(10000).unwrap(), RunOutcome::Exited);
    assert_eq!(s.instructions(), 5571);
    assert_eq!(s.kernel().ipc_records().len(), 25);
    assert_eq!(s.kernel().live_handle_count(), 0);
    assert_eq!(s.kernel().live_transfer_memory_count(), 0);
    assert_eq!(s.memory().lent_bytes(), 0);
    assert_eq!(s.memory().read_u64(LOAD_BASE + 0x4040).unwrap(), 2);
    let mut pixels = [0; 512];
    s.memory().read(LOAD_BASE + 0x5800, &mut pixels).unwrap();
    for y in 0..8 {
        for x in 0..16 {
            let expected = if (4..12).contains(&x) && (2..6).contains(&y) {
                [0, 255, 0, 255]
            } else if x < 8 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            assert_eq!(&pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4], &expected);
        }
    }
}
