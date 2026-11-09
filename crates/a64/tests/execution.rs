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
fn load_store_pair_pre_post_index_and_offsets() {
    let mut memory = AddressSpace::default();
    // Map code at CODE, and data stack at 0x8000
    // We will test STP x0, x1, [sp, #-16]! (pre-index)
    // STP word: 10 101 0 011 0 imm7=1111110(-2) rt2=00001 rn=11111 rt1=00000 -> 0xa9bf07e0
    // Then LDP x2, x3, [sp], #16 (post-index)
    // LDP word: 10 101 0 001 1 imm7=0000010(+2) rt2=00011 rn=11111 rt1=00010 -> 0xa8c20fe2
    // SVC 0x1: 0xd4000021
    let program = vec![
        0xe0, 0x07, 0xbf, 0xa9, // stp x0, x1, [sp, #-16]!
        0xe2, 0x0f, 0xc1, 0xa8, // ldp x2, x3, [sp], #16
        0x21, 0x00, 0x00, 0xd4, // svc #1
    ];
    memory.map(CODE, program, Permissions::READ_EXECUTE).unwrap();
    memory.map(0x7000, vec![0; 8192], Permissions::READ_WRITE).unwrap();
    let mut cpu = Cpu::new(CODE, 0x8000);
    cpu.x[0] = 0x1122334455667788;
    cpu.x[1] = 0x99aabbccddeeff00;

    assert_eq!(next_svc(&mut cpu, &mut memory), (1, 3));
    assert_eq!(cpu.sp, 0x8000); // Pre-decremented then post-incremented
    assert_eq!(cpu.x[2], 0x1122334455667788);
    assert_eq!(cpu.x[3], 0x99aabbccddeeff00);
    // Check values stored in memory at 0x7ff0 and 0x7ff8
    assert_eq!(memory.read_u64(0x7ff0).unwrap(), 0x1122334455667788);
    assert_eq!(memory.read_u64(0x7ff8).unwrap(), 0x99aabbccddeeff00);
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

#[test]
fn llvm_shifted_register_vectors_cover_widths_shifts_flags_and_zero_register() {
    let bytes = include_bytes!("../../../examples/a64/shifted-add.bin");
    let vectors = [
        (10, 3, 22, None),
        (u64::MAX, 0x80000000, 0, Some((false, true, true, false))),
        (0, u64::MAX, 1, Some((false, false, false, false))),
        (12, 19, 999, None),
        (u64::MAX, 3, 0x80000000, None),
        (0x100000000, 1, 0xffffffff, None),
        (
            0x7fffffffffffffff,
            1,
            0x8000000000000000,
            Some((true, false, false, true)),
        ),
        (0x80000000, 1, 0x7fffffff, Some((false, false, true, true))),
        (10, 0x8000000000000000, 11, None),
        (0, 0x80000000, 1, None),
    ];
    for (i, (lhs, rhs, want, flags)) in vectors.into_iter().enumerate() {
        let (mut cpu, mut memory) = machine(&bytes[i * 4..i * 4 + 4]);
        cpu.x[0] = 999;
        cpu.x[1] = lhs;
        cpu.x[2] = rhs;
        cpu.sp = 0x81234567;
        let saved = cpu.nzcv;
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.x[0], want, "vector {i}");
        assert_eq!(cpu.sp, 0x81234567);
        if let Some((negative, zero, carry, overflow)) = flags {
            assert_eq!(
                cpu.nzcv,
                Nzcv {
                    negative,
                    zero,
                    carry,
                    overflow
                }
            );
        } else {
            assert_eq!(cpu.nzcv, saved);
        }
    }
}
#[test]
fn reserved_shifted_register_forms_preserve_cpu_and_pc() {
    for word in [0x8bc20020u32, 0x0b028020] {
        // ROR encoding, W32 shift32
        let (mut cpu, mut memory) = machine(&word.to_le_bytes());
        let saved = cpu.clone();
        assert!(matches!(
            cpu.step(&mut memory),
            Err(ExecutionError::ReservedEncoding { .. })
        ));
        assert_eq!(cpu, saved);
    }
}

#[test]
fn test_adr_and_adrp_address_calculation() {
    let mut cpu = Cpu::default();
    cpu.pc = 0x8000_1008;
    let mut mem = AddressSpace::default();
    // ADR x0, #16 -> offset +16:
    // immlo = (16 & 3) = 0, immhi = (16 >> 2) = 4 -> 0x1000_0080
    let adr_word = 0x1000_0080u32;
    // ADRP x1, #0x2000 -> imm = 2 pages:
    // immlo = 2 & 3 = 2, immhi = (2 >> 2) = 0 -> bits [30:29] = 2 -> 0x9000_0000 | (2 << 29) | (0 << 5) | 1 = 0xd000_0001
    let adrp_word = 0xd000_0001u32;

    let mut code_bytes = vec![0u8; 0x2000];
    code_bytes[8..12].copy_from_slice(&adr_word.to_le_bytes());
    code_bytes[12..16].copy_from_slice(&adrp_word.to_le_bytes());
    mem.map(0x8000_1000, code_bytes, Permissions::READ_EXECUTE).unwrap();

    let event = cpu.step(&mut mem).unwrap();
    assert_eq!(event, StepEvent::Continue);
    assert_eq!(cpu.x[0], 0x8000_1018); // 0x8000_1008 + 16

    let event = cpu.step(&mut mem).unwrap();
    assert_eq!(event, StepEvent::Continue);
    // (0x8000_100c & !0xfff) + (2 << 12) = 0x8000_1000 + 0x2000 = 0x8000_3000
    assert_eq!(cpu.x[1], 0x8000_3000);
}
