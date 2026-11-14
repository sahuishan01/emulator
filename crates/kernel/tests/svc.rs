use emulator_a64::Cpu;
use emulator_kernel::{
    Kernel, KernelError, KernelEvent, RESULT_INVALID_ADDRESS, SVC_EXIT_PROCESS,
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
    assert_eq!(memory.read_u32(0x1010).unwrap(), 0x02); // Static code
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

    // Query 7: query_address beyond address space (>= 1 << 39)
    cpu.x[0] = 0x1000;
    cpu.x[2] = 1_u64 << 39;
    assert_eq!(
        kernel
            .dispatch(SVC_QUERY_MEMORY, &mut cpu, &mut memory)
            .unwrap(),
        KernelEvent::Continue
    );
    assert_eq!(cpu.x[0], RESULT_INVALID_ADDRESS);

    // Query 8: Sequential traversal loop like rtld
    let mut current_addr = 0u64;
    let mut visited_code = false;
    let mut visited_data = false;
    while current_addr < 0x8004_0000 {
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
        assert!(size > 0);
        current_addr = base + size;
    }
    assert!(visited_code);
    assert!(visited_data);
}
