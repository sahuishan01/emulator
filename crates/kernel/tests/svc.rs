use emulator_a64::Cpu;
use emulator_kernel::{
    Kernel, KernelError, KernelEvent, SVC_EXIT_PROCESS, SVC_OUTPUT_DEBUG_STRING,
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
