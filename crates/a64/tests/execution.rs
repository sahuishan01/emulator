use emulator_a64::{Cpu, ExecutionError, Nzcv, StepEvent};
use emulator_memory::{Access, AddressSpace, MemoryError, Permissions};

const CODE: u64 = 0x10_0000;

#[test]
fn thread_local_register_is_guest_owned_and_other_system_registers_fault() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/thread-local.bin"));
    cpu.tpidrro_el0 = 0x7ffe_0200;
    let flags = cpu.nzcv;
    assert_eq!(next_svc(&mut cpu, &mut memory), (0x42, 3));
    assert_eq!(cpu.x[0], 0x7ffe_0200);
    assert_eq!(cpu.sp, 0x8000);
    assert_eq!(cpu.nzcv, flags);
    let saved = cpu.clone();
    assert!(matches!(
        cpu.step(&mut memory),
        Err(ExecutionError::UnsupportedInstruction { .. })
    ));
    assert_eq!(cpu, saved);
}

fn machine(program: &[u8]) -> (Cpu, AddressSpace) {
    let mut memory = AddressSpace::default();
    memory
        .map(CODE, program.to_vec(), Permissions::READ_EXECUTE)
        .unwrap();
    memory
        .map(0x2000, vec![0; 4096], Permissions::READ_WRITE)
        .unwrap();
    (Cpu::new(CODE, 0x8000), memory)
}

fn next_svc(cpu: &mut Cpu, memory: &mut AddressSpace) -> (u16, u64) {
    for steps in 1..=100 {
        match cpu.step(memory).unwrap() {
            StepEvent::Continue => {}
            StepEvent::SupervisorCall { immediate } => return (immediate, steps),
        }
    }
    panic!("fixture failed to reach its next SVC checkpoint");
}

#[test]
fn assembled_loop_call_return_and_guest_memory() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/demo.bin"));
    assert_eq!(next_svc(&mut cpu, &mut memory), (0x42, 24));
    assert_eq!(cpu.x[0], 35);
    assert_eq!(cpu.x[1], 0);
    assert_eq!(cpu.x[3], 42);
    assert_eq!(cpu.x[30], CODE + 36);
    assert_eq!(cpu.pc, CODE + 40);
    assert_eq!(memory.read_u64(0x2000).unwrap(), 35);
}

#[test]
fn arithmetic_flags_cover_carry_borrow_and_signed_overflow_at_both_widths() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/flags.bin"));
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 1);
    assert_eq!(cpu.x[0], u64::MAX);
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: true,
            zero: false,
            carry: false,
            overflow: false
        }
    );
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 2);
    assert_eq!(cpu.x[0], 0);
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: false,
            zero: true,
            carry: true,
            overflow: false
        }
    );
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 3);
    assert_eq!(cpu.x[2], 0x7fff_ffff_ffff_ffff);
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: false,
            zero: false,
            carry: true,
            overflow: true
        }
    );
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 5);
    assert_eq!(cpu.x[2], 0x8000_0000_0000_0000);
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: true,
            zero: false,
            carry: false,
            overflow: true
        }
    );
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 4);
    assert_eq!(cpu.x[2], 0x7fff_ffff);
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: false,
            zero: false,
            carry: true,
            overflow: true
        }
    );
}

#[test]
fn zero_register_stack_pointer_and_writes_to_w_registers_are_distinct() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/registers.bin"));
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 0x42);
    assert_eq!(cpu.x[0], 0x1234);
    assert_eq!(cpu.sp, 0x8010);
    assert_eq!(cpu.x[1], 0x8010);
    assert!(!cpu.nzcv.zero);
}

#[test]
fn load_store_width_and_scaled_offsets_preserve_neighboring_bytes() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/memory.bin"));
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 0x42);
    assert_eq!(cpu.x[2], 0x1234_0000_0000_0000);
    assert_eq!(cpu.x[4], 0xbeef_0000);
    assert_eq!(memory.read_u64(0x2008).unwrap(), 0x1234_0000_0000_0000);
    assert_eq!(memory.read_u32(0x2004).unwrap(), 0xbeef_0000);
    assert_eq!(memory.read_u32(0x2000).unwrap(), 0);
}

#[test]
fn compare_branches_use_the_selected_register_width() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/compare-branch.bin"));
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 0x42);
    assert_eq!(cpu.x[0], 1 << 32);
    assert_eq!(cpu.x[1], 42);
}

#[test]
fn backward_unconditional_branch_executes_a_bounded_loop() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/branches.bin"));
    assert_eq!(next_svc(&mut cpu, &mut memory), (0x42, 10));
    assert_eq!(cpu.x[0], 0);
}

#[test]
fn unsupported_and_reserved_encodings_do_not_advance_or_modify_cpu() {
    for word in [0_u32, 0x52c0_0000] {
        let (mut cpu, mut memory) = machine(&word.to_le_bytes());
        cpu.x[0] = 123;
        let before = cpu.clone();
        let error = cpu.step(&mut memory).unwrap_err();
        match word {
            0 => assert!(matches!(
                error,
                ExecutionError::UnsupportedInstruction { pc: CODE, word: 0 }
            )),
            _ => assert!(matches!(
                error,
                ExecutionError::ReservedEncoding { pc: CODE, .. }
            )),
        }
        assert_eq!(cpu, before);
    }
}

#[test]
fn failed_store_leaves_cpu_and_read_only_data_unchanged() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/store.bin"));
    memory
        .map(0x4000, vec![0xaa; 8], Permissions::READ)
        .unwrap();
    cpu.x[0] = 0;
    cpu.x[1] = 0x4000;
    let before = cpu.clone();
    assert!(matches!(
        cpu.step(&mut memory),
        Err(ExecutionError::Memory(MemoryError::PermissionDenied {
            access: Access::Write,
            ..
        }))
    ));
    assert_eq!(cpu, before);
    assert_eq!(memory.read_u64(0x4000).unwrap(), 0xaaaa_aaaa_aaaa_aaaa);
}

#[test]
fn failed_load_does_not_overwrite_destination_register() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/load.bin"));
    cpu.x[0] = 42;
    cpu.x[1] = 0x9000;
    let before = cpu.clone();
    assert!(matches!(
        cpu.step(&mut memory),
        Err(ExecutionError::Memory(MemoryError::Unmapped { .. }))
    ));
    assert_eq!(cpu, before);
}

#[test]
fn pc_relative_addresses_and_move_wide_preserve_width_semantics() {
    let (mut cpu, mut memory) = machine(include_bytes!("../../../examples/a64/addressing.bin"));
    assert_eq!(next_svc(&mut cpu, &mut memory).0, 0x42);
    assert_eq!(cpu.x[0], CODE + 4096);
    assert_eq!(cpu.x[1], CODE + 4096);
    assert_eq!(cpu.x[2], 0xabcdffff);
}
