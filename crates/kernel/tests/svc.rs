use emulator_a64::Cpu;
use emulator_kernel::{
    Kernel, KernelError, KernelEvent, RESULT_INVALID_ADDRESS, RESULT_TIMEOUT, SVC_EXIT_PROCESS,
    SVC_OUTPUT_DEBUG_STRING, SVC_QUERY_MEMORY,
};
use emulator_memory::{AddressSpace, Permissions};

#[test]
fn debug_output_reads_guest_bytes_and_returns_success_only_after_validation() {
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, vec![b'h', b'i', 0xff, 0], Permissions::READ)
        .unwrap();
    let mut cpu = Cpu::new(0, 0);
    cpu.x[0] = 0x1000;
    cpu.x[1] = 4;
    let mut kernel = Kernel::default();
    assert_eq!(
        kernel
            .dispatch(SVC_OUTPUT_DEBUG_STRING, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(kernel.debug_records(), &[vec![b'h', b'i', 0xff, 0]]);
}

#[test]
fn invalid_pointer_and_oversized_output_do_not_create_partial_records() {
    let mut memory = AddressSpace::default();
    let mut cpu = Cpu::new(0, 0);
    cpu.x[0] = 0x1000;
    cpu.x[1] = 4;
    let original = cpu.clone();
    let mut kernel = Kernel::default();
    assert!(matches!(
        kernel.dispatch(SVC_OUTPUT_DEBUG_STRING, &mut cpu, &mut memory),
        Err(KernelError::InvalidDebugBuffer(_))
    ));
    assert_eq!(cpu, original);
    cpu.x[1] = u64::MAX;
    assert_eq!(
        kernel.dispatch(SVC_OUTPUT_DEBUG_STRING, &mut cpu, &mut memory),
        Err(KernelError::DebugLimit)
    );
    assert!(kernel.debug_records().is_empty());
}

#[test]
fn exit_process_has_no_return_value_and_unknown_calls_are_not_stubbed() {
    let mut memory = AddressSpace::default();
    let mut cpu = Cpu::new(0, 0);
    cpu.x[0] = 123;
    let mut kernel = Kernel::default();
    assert_eq!(
        kernel.dispatch(0x42, &mut cpu, &mut memory),
        Err(KernelError::UnsupportedSupervisorCall(0x42))
    );
    assert_eq!(cpu.x[0], 123);
    assert_eq!(
        kernel
            .dispatch(SVC_EXIT_PROCESS, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::ProcessExited
    );
    assert!(kernel.exited());
    assert_eq!(cpu.x[0], 123);
    assert_eq!(
        kernel.dispatch(SVC_EXIT_PROCESS, &mut cpu, &mut memory),
        Err(KernelError::ProcessAlreadyExited)
    );
}

#[test]
fn query_memory_returns_accurate_descriptors_and_page_aligned_traversal() {
    let mut memory = AddressSpace::default();
    // Region 1: 0x1000..0x2000 (output buffer for SVC)
    memory
        .map(0x1000, vec![0; 4096], Permissions::READ_WRITE)
        .unwrap();
    // Region 2: 0x8000_0000..0x8001_0000 (code segment, R-X)
    memory
        .map(0x8000_0000, vec![0; 0x10000], Permissions::READ_EXECUTE)
        .unwrap();
    // Region 3: 0x8001_0000..0x8002_0000 (data segment, RW)
    memory
        .map(0x8001_0000, vec![0; 0x10000], Permissions::READ_WRITE)
        .unwrap();
    // Region 4: 0x8003_0000..0x8004_0000 (reserved, Inaccessible)
    memory.reserve(0x8003_0000, 0x10000).unwrap();

    let mut kernel = Kernel::default();
    let mut cpu = Cpu::new(0, 0);

    // Query 1: address 0 (unmapped gap before 0x1000)
    cpu.x[0] = 0x1000;
    cpu.x[2] = 0;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 0);
    let base = memory.read_u64(0x1000).unwrap();
    let size = memory.read_u64(0x1008).unwrap();
    let state = memory.read_u32(0x1010).unwrap();
    let perm = memory.read_u32(0x1018).unwrap();
    assert_eq!(base, 0);
    assert_eq!(size, 0x1000);
    assert_eq!(state, 0); // Free
    assert_eq!(perm, 0); // None

    // Query 2: address 0x8000_0000 (code segment, R-X)
    cpu.x[0] = 0x1000;
    cpu.x[2] = 0x8000_0000;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x8000_0000);
    assert_eq!(memory.read_u64(0x1008).unwrap(), 0x10000);
    assert_eq!(memory.read_u32(0x1010).unwrap(), 0x03); // CodeStatic
    assert_eq!(memory.read_u32(0x1018).unwrap(), 5); // ReadExecute

    // Query 3: address 0x8001_5000 (data segment, RW)
    cpu.x[0] = 0x1000;
    cpu.x[2] = 0x8001_5000;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x8001_0000);
    assert_eq!(memory.read_u64(0x1008).unwrap(), 0x10000);
    assert_eq!(memory.read_u32(0x1018).unwrap(), 3); // ReadWrite

    // Query 4: address 0x8002_5000 (gap between data and reserve)
    cpu.x[0] = 0x1000;
    cpu.x[2] = 0x8002_5000;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x8002_0000);
    assert_eq!(memory.read_u64(0x1008).unwrap(), 0x10000);
    assert_eq!(memory.read_u32(0x1010).unwrap(), 0); // Free
    assert_eq!(memory.read_u32(0x1018).unwrap(), 0); // None

    // Query 5: address 0x8003_0000 (reserved, Inaccessible)
    cpu.x[0] = 0x1000;
    cpu.x[2] = 0x8003_0000;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x8003_0000);
    assert_eq!(memory.read_u64(0x1008).unwrap(), 0x10000);
    assert_eq!(memory.read_u32(0x1010).unwrap(), 0x10); // Inaccessible
    assert_eq!(memory.read_u32(0x1018).unwrap(), 0); // None

    // Query 6: out_meminfo buffer unaligned
    cpu.x[0] = 0x1001;
    cpu.x[2] = 0x8000_0000;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], RESULT_INVALID_ADDRESS);

    // Query 7: query_address beyond address space (>= 1 << 39) returns fake Inaccessible block with success
    cpu.x[0] = 0x1000;
    cpu.x[2] = 1_u64 << 39;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 0);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 1_u64 << 39);
    assert_eq!(memory.read_u64(0x1008).unwrap(), 0u64.wrapping_sub(1_u64 << 39));
    assert_eq!(memory.read_u32(0x1010).unwrap(), 0);
    assert_eq!(memory.read_u32(0x1018).unwrap(), 0);

    // Query 8: verifies page_info output pointer is populated with 0
    memory.write_u32(0x1030, 0xdead_beef).unwrap();
    cpu.x[0] = 0x1000;
    cpu.x[1] = 0x1030; // page_info output pointer
    cpu.x[2] = 0x8000_0000;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 0);
    assert_eq!(memory.read_u32(0x1030).unwrap(), 0);

    // Query 9: Sequential traversal loop like rtld across the entire address space
    let mut current_addr = 0u64;
    let mut visited_code = false;
    let mut visited_data = false;
    let mut query_count = 0;
    loop {
        cpu.x[0] = 0x1000;
        cpu.x[2] = current_addr;
        assert_eq!(
            kernel
                .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
                .unwrap(),
            KernelEvent::Continue
        );
        assert_eq!(cpu.x[0], 0);
        let base = memory.read_u64(0x1000).unwrap();
        let size = memory.read_u64(0x1008).unwrap();
        let perm = memory.read_u32(0x1018).unwrap();
        if base == 0x8000_0000 && perm == 5 {
            visited_code = true;
        }
        if base == 0x8001_0000 && perm == 3 {
            visited_data = true;
        }
        query_count += 1;
        let next_addr = base.wrapping_add(size);
        if next_addr <= current_addr || base >= (1_u64 << 39) {
            break;
        }
        current_addr = next_addr;
    }
    assert!(visited_code);
    assert!(visited_data);
    assert!(query_count >= 5);
}

#[test]
fn set_heap_size_allocates_and_queries_memory_state() {
    use emulator_kernel::{DEFAULT_HEAP_BASE, RESULT_INVALID_SIZE, SVC_SET_HEAP_SIZE};
    let mut memory = AddressSpace::default();
    memory.map(0x1000, vec![0; 4096], Permissions::READ_WRITE).unwrap();

    let mut kernel = Kernel::default();
    let mut cpu = Cpu::new(0, 0);

    // Initial query at HEAP_BASE -> free (state 0)
    cpu.x[0] = 0x1000;
    cpu.x[2] = DEFAULT_HEAP_BASE;
    assert_eq!(
        kernel.dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    let state = memory.read_u32(0x1010).unwrap();
    assert_eq!(state, 0); // Free unmapped gap

    // Invalid non-page-aligned size
    cpu.x[1] = 4095;
    assert_eq!(
        kernel.dispatch(SVC_SET_HEAP_SIZE, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], RESULT_INVALID_SIZE);

    // Allocate 2 MiB heap
    cpu.x[1] = 0x20_0000;
    assert_eq!(
        kernel.dispatch(SVC_SET_HEAP_SIZE, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], DEFAULT_HEAP_BASE);

    // Query allocated heap memory
    cpu.x[0] = 0x1000;
    cpu.x[2] = DEFAULT_HEAP_BASE + 0x1000;
    assert_eq!(
        kernel.dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    let base = memory.read_u64(0x1000).unwrap();
    let size = memory.read_u64(0x1008).unwrap();
    let state = memory.read_u32(0x1010).unwrap();
    let perm = memory.read_u32(0x1018).unwrap();
    assert_eq!(base, DEFAULT_HEAP_BASE);
    assert_eq!(size, 0x20_0000);
    assert_eq!(state, 0x03); // MemoryState::Normal / Heap
    assert_eq!(perm, 3); // Read | Write

    // Write to heap
    assert!(memory.write_u32(DEFAULT_HEAP_BASE, 0x12345678).is_ok());
    assert_eq!(memory.read_u32(DEFAULT_HEAP_BASE).unwrap(), 0x12345678);

    // Shrink heap to 1 MiB
    cpu.x[1] = 0x10_0000;
    assert_eq!(
        kernel.dispatch(SVC_SET_HEAP_SIZE, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], DEFAULT_HEAP_BASE);
}

#[test]
fn system_tick_thread_id_futex_and_processor_primitives() {
    use emulator_kernel::{
        SVC_ARBITRATE_LOCK, SVC_ARBITRATE_UNLOCK, SVC_BREAK,
        SVC_GET_CURRENT_PROCESSOR_NUMBER, SVC_GET_INFO, SVC_GET_PROCESS_ID,
        SVC_GET_SYSTEM_TICK, SVC_GET_THREAD_ID, SVC_SET_MEMORY_ATTRIBUTE,
        SVC_SET_MEMORY_PERMISSION, SVC_SIGNAL_PROCESS_WIDE_KEY,
        SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC,
    };
    let mut memory = AddressSpace::default();
    memory.map(0x1000, vec![0; 4096], Permissions::READ_WRITE).unwrap();

    let mut kernel = Kernel::default();
    let mut cpu = Cpu::new(0, 0);

    // Current processor number
    assert_eq!(
        kernel.dispatch(SVC_GET_CURRENT_PROCESSOR_NUMBER, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);

    // System tick
    assert_eq!(
        kernel.dispatch(SVC_GET_SYSTEM_TICK, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );

    // Set memory attribute
    cpu.x[0] = 0x1000;
    cpu.x[1] = 4096;
    cpu.x[2] = 0;
    cpu.x[3] = 0;
    assert_eq!(
        kernel.dispatch(SVC_SET_MEMORY_ATTRIBUTE, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);

    // Thread ID
    cpu.x[0] = 0x1000;
    cpu.x[1] = 0xffff_8000; // current thread
    assert_eq!(
        kernel.dispatch(SVC_GET_THREAD_ID, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 1);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 1);

    // Process ID
    cpu.x[0] = 0x1000;
    cpu.x[1] = 0xffff_8001;
    assert_eq!(
        kernel.dispatch(SVC_GET_PROCESS_ID, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 1);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 1);

    // GetInfo (CoreMask)
    cpu.x[0] = 0x1000;
    cpu.x[1] = 0; // InfoType_CoreMask
    cpu.x[2] = 0xffff_8001;
    cpu.x[3] = 0;
    assert_eq!(
        kernel.dispatch(SVC_GET_INFO, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 7);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 7);

    // GetInfo (AslrRegionAddress & AslrRegionSize)
    kernel.set_aslr_region(0x10_0000, 0x7f_ff00_0000);
    cpu.x[0] = 0x1000;
    cpu.x[1] = 12; // InfoType_AslrRegionAddress
    cpu.x[2] = 0xffff_8001;
    cpu.x[3] = 0;
    assert_eq!(
        kernel.dispatch(SVC_GET_INFO, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 0x10_0000);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x10_0000);

    cpu.x[0] = 0x1000;
    cpu.x[1] = 13; // InfoType_AslrRegionSize
    cpu.x[2] = 0xffff_8001;
    cpu.x[3] = 0;
    assert_eq!(
        kernel.dispatch(SVC_GET_INFO, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 0x7f_ff00_0000);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x7f_ff00_0000);

    // GetInfo (ProgramId)
    kernel.set_program_id(0x0100_1520_0000_1000);
    cpu.x[0] = 0x1000;
    cpu.x[1] = 18; // InfoType_ProgramId
    cpu.x[2] = 0xffff_8001;
    cpu.x[3] = 0;
    assert_eq!(
        kernel.dispatch(SVC_GET_INFO, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(cpu.x[1], 0x0100_1520_0000_1000);
    assert_eq!(memory.read_u64(0x1000).unwrap(), 0x0100_1520_0000_1000);

    // Set memory permission
    cpu.x[0] = 0x1000;
    cpu.x[1] = 4096;
    cpu.x[2] = 1; // Read only
    assert_eq!(
        kernel.dispatch(SVC_SET_MEMORY_PERMISSION, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);

    // Restore Read-Write permission for subsequent tests
    cpu.x[0] = 0x1000;
    cpu.x[1] = 4096;
    cpu.x[2] = 3; // ReadWrite
    assert_eq!(
        kernel.dispatch(SVC_SET_MEMORY_PERMISSION, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);

    // Futex lock / unlock
    memory.write_u32(0x1000, 0).unwrap();
    cpu.x[0] = 1; // thread_handle
    cpu.x[1] = 0x1000; // mutex_addr
    cpu.x[2] = 0; // expected tag
    assert_eq!(
        kernel.dispatch(SVC_ARBITRATE_LOCK, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(memory.read_u32(0x1000).unwrap(), 1); // mutex tag set to owner handle

    cpu.x[0] = 0x1000;
    assert_eq!(
        kernel.dispatch(SVC_ARBITRATE_UNLOCK, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert_eq!(memory.read_u32(0x1000).unwrap(), 0); // unlocked

    // Condition variable wait (timeout 0) / signal
    cpu.x[0] = 0x1000; // mutex_addr
    cpu.x[1] = 0x2000; // key_addr
    cpu.x[2] = 1;      // thread_handle / tag
    cpu.x[3] = 0;      // timeout_ns = 0 -> immediate timeout
    assert_eq!(
        kernel.dispatch(SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], RESULT_TIMEOUT);

    cpu.x[0] = 0x2000; // key_addr
    cpu.x[1] = 1;      // count
    assert_eq!(
        kernel.dispatch(SVC_SIGNAL_PROCESS_WIDE_KEY, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);

    // Break
    cpu.x[0] = 0x42;
    cpu.x[1] = 0x100;
    cpu.x[2] = 0x200;
    assert_eq!(
        kernel.dispatch(SVC_BREAK, &mut cpu, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], 0);
    assert!(kernel.debug_records().iter().any(|r| String::from_utf8_lossy(r).contains("svcBreak")));
}

#[test]
fn test_condvar_wait_signal_requeue_cycle() {
    let mut memory = AddressSpace::default();
    memory.map(0x1000, vec![0; 4096], Permissions::READ_EXECUTE).unwrap();
    memory.map(0x4000, vec![0; 4096], Permissions::READ_WRITE).unwrap();
    let mut kernel = Kernel::default();
    let mut cpu0 = Cpu::new(0x1000, 0x5000);

    // Create a second thread
    cpu0.x[1] = 0x1000; // entry
    cpu0.x[2] = 42; // arg
    cpu0.x[3] = 0x5000; // stack
    cpu0.x[4] = 44; // priority (same as main)
    cpu0.x[5] = 0; // core
    assert_eq!(
        kernel.dispatch(emulator_kernel::SVC_CREATE_THREAD, &mut cpu0, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    let child_handle = cpu0.x[1] as u32;
    cpu0.x[0] = child_handle as u64;
    assert_eq!(
        kernel.dispatch(emulator_kernel::SVC_START_THREAD, &mut cpu0, &mut memory).unwrap(),
        KernelEvent::Continue
    );

    // Thread 0 locks mutex at 0x4000
    cpu0.x[0] = 1; // owner tag
    cpu0.x[1] = 0x4000; // mutex_addr
    cpu0.x[2] = 1; // tag
    assert_eq!(
        kernel.dispatch(emulator_kernel::SVC_ARBITRATE_LOCK, &mut cpu0, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(memory.read_u32(0x4000).unwrap(), 1);

    // Thread 0 waits on condvar at 0x4010 releasing mutex at 0x4000
    cpu0.x[0] = 0x4000; // mutex_addr
    cpu0.x[1] = 0x4010; // key_addr
    cpu0.x[2] = 1; // tag
    cpu0.x[3] = u64::MAX; // -1 (infinite timeout)
    assert_eq!(
        kernel.dispatch(emulator_kernel::SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC, &mut cpu0, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    // Mutex should now be unlocked (0)
    assert_eq!(memory.read_u32(0x4000).unwrap(), 0);

    // Thread 0 is blocked waiting on condvar, so scheduler runs Thread 1
    let mut cpu1 = Cpu::default();
    assert_eq!(kernel.prepare_execution(&mut cpu1), emulator_kernel::ScheduleOutcome::Ready);
    assert_eq!(kernel.current_thread_id(), 1);

    // Thread 1 locks mutex at 0x4000
    cpu1.x[0] = 2; // owner tag
    cpu1.x[1] = 0x4000; // mutex_addr
    cpu1.x[2] = 2; // tag
    assert_eq!(
        kernel.dispatch(emulator_kernel::SVC_ARBITRATE_LOCK, &mut cpu1, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(memory.read_u32(0x4000).unwrap(), 2);

    // Thread 1 signals condvar at 0x4010
    // Since mutex 0x4000 is held by Thread 1 (tag 2), Thread 0 should be requeued to mutex 0x4000
    // and mutex 0x4000 should get the waiter flag (0x4000_0000)
    cpu1.x[0] = 0x4010;
    cpu1.x[1] = 1; // wake 1
    assert_eq!(
        kernel.dispatch(emulator_kernel::SVC_SIGNAL_PROCESS_WIDE_KEY, &mut cpu1, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(memory.read_u32(0x4000).unwrap(), 2 | 0x4000_0000);

    // Thread 1 unlocks mutex at 0x4000
    // Thread 0 should be unblocked and assigned ownership of mutex 0x4000!
    cpu1.x[0] = 0x4000;
    assert_eq!(
        kernel.dispatch(emulator_kernel::SVC_ARBITRATE_UNLOCK, &mut cpu1, &mut memory).unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(memory.read_u32(0x4000).unwrap(), 1); // Mutex owned by Thread 0

    // Switch to Thread 0
    // Thread 1 yields or quantum exhausted
    for _ in 0..64 {
        kernel.instruction_completed().unwrap();
    }
    assert_eq!(kernel.prepare_execution(&mut cpu0), emulator_kernel::ScheduleOutcome::Ready);
    assert_eq!(kernel.current_thread_id(), 0);
}
