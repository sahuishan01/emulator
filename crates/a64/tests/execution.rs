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

#[test]
fn test_logical_shifted_register_and_mov() {
    let mut cpu = Cpu::default();
    cpu.pc = 0x8000_1000;
    cpu.x[1] = 0x1234_5678_9abc_def0;
    cpu.x[2] = 0x0000_0000_ffff_ffff;

    let mut mem = AddressSpace::default();

    // MOV x0, x1  -> ORR x0, xzr, x1: 0xaa0103e0
    let mov_x0_x1 = 0xaa0103e0u32;
    // AND x3, x1, x2: opc=00, n=0 -> 0x8a020023
    // sf=1, opc=00, 01010, shift=00, N=0, Rm=2, imm6=0, Rn=1, Rd=3
    // (1<<31) | (0xa<<24) | (2<<16) | (1<<5) | 3 = 0x8a020023
    let and_x3_x1_x2 = 0x8a020023u32;
    // EOR x4, x1, x2: opc=10, n=0 -> 0xca020024
    let eor_x4_x1_x2 = 0xca020024u32;

    let mut code_bytes = vec![0u8; 0x1000];
    code_bytes[0..4].copy_from_slice(&mov_x0_x1.to_le_bytes());
    code_bytes[4..8].copy_from_slice(&and_x3_x1_x2.to_le_bytes());
    code_bytes[8..12].copy_from_slice(&eor_x4_x1_x2.to_le_bytes());
    mem.map(0x8000_1000, code_bytes, Permissions::READ_EXECUTE).unwrap();

    // Step 1: MOV
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 0x1234_5678_9abc_def0);

    // Step 2: AND
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[3], 0x0000_0000_9abc_def0);

    // Step 3: EOR
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[4], 0x1234_5678_6543_210f);
}

#[test]
fn test_bitfield_and_exclusive_load_store() {
    let mut cpu = Cpu::default();
    cpu.pc = 0x8000_1000;
    cpu.x[1] = 0x0123_4567_89ab_cdef;
    cpu.x[2] = 0x8000_2000; // memory address

    let mut mem = AddressSpace::default();

    // UBFX x0, x1, #8, #16 -> UBFM x0, x1, #8, #23 (immr=8, imms=23)
    // 1(sf) 10(opc) 100110 1(N) 001000(immr=8) 010111(imms=23) 00001(Rn=1) 00000(Rd=0)
    // (1<<31) | (2<<29) | (0x26<<23) | (1<<22) | (8<<16) | (23<<10) | (1<<5) | 0 = 0xd3485c20
    let ubfx = 0xd3485c20u32;

    // SBFX x3, x1, #0, #8 -> SBFM x3, x1, #0, #7 (immr=0, imms=7)
    // 1(sf) 00(opc) 100110 1(N) 000000(immr=0) 000111(imms=7) 00001(Rn=1) 00011(Rd=3)
    // (1<<31) | (0<<29) | (0x26<<23) | (1<<22) | (0<<16) | (7<<10) | (1<<5) | 3 = 0x93401c23
    let sbfx = 0x93401c23u32;

    // STXR w4, x1, [x2] -> 32-bit status in w4, 64-bit store of x1 to [x2]
    // 1(sf) 0001000 0(store) 0 00100(Rs=4) 011111 00010(Rn=2) 00001(Rt=1)
    // (1<<31) | (0x08<<24) | (0<<22) | (4<<16) | (0x1f<<10) | (2<<5) | 1 = 0xc8047c41
    let stxr = 0xc8047c41u32;

    // LDXR x5, [x2] -> 64-bit load from [x2] to x5
    // 1(sf) 0001000 0(load) 1 11111(Rs=31) 011111 00010(Rn=2) 00101(Rt=5)
    // (1<<31) | (0x08<<24) | (1<<22) | (31<<16) | (0x1f<<10) | (2<<5) | 5 = 0xc85f7c45
    let ldxr = 0xc85f7c45u32;

    let mut code_bytes = vec![0u8; 0x1000];
    code_bytes[0..4].copy_from_slice(&ubfx.to_le_bytes());
    code_bytes[4..8].copy_from_slice(&sbfx.to_le_bytes());
    code_bytes[8..12].copy_from_slice(&stxr.to_le_bytes());
    code_bytes[12..16].copy_from_slice(&ldxr.to_le_bytes());
    mem.map(0x8000_1000, code_bytes, Permissions::READ_EXECUTE).unwrap();
    mem.map(0x8000_2000, vec![0u8; 0x1000], Permissions::READ_WRITE).unwrap();

    // Step 1: UBFX
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 0xabcd); // bits [23:8] of 0x0123_4567_89ab_cdef are 0xabcd

    // Step 2: SBFX
    cpu.step(&mut mem).unwrap();
    // bits [7:0] of x1 is 0xef (-17 signed)
    assert_eq!(cpu.x[3], 0xffff_ffff_ffff_ffef);

    // Step 3: STXR
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[4], 0); // exclusive store status 0 (success)
    assert_eq!(mem.read_u64(0x8000_2000).unwrap(), 0x0123_4567_89ab_cdef);

    // Step 4: LDXR
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[5], 0x0123_4567_89ab_cdef);
}

#[test]
fn test_branch_register_and_blr_retail_crt0() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);
    // Instruction 0x1000: BLR X1 (0xd63f0020) - matches Pokémon Legends: Z-A rtld!
    let blr_x1 = 0xd63f0020u32;
    // Instruction 0x2000: BR X2 (0xd61f0040)
    let br_x2 = 0xd61f0040u32;
    // Instruction 0x3000: RET (0xd65f03c0)
    let ret = 0xd65f03c0u32;

    let mut code1 = vec![0u8; 0x1000];
    code1[0..4].copy_from_slice(&blr_x1.to_le_bytes());
    mem.map(0x1000, code1, Permissions::READ_EXECUTE).unwrap();

    let mut code2 = vec![0u8; 0x1000];
    code2[0..4].copy_from_slice(&br_x2.to_le_bytes());
    mem.map(0x2000, code2, Permissions::READ_EXECUTE).unwrap();

    let mut code3 = vec![0u8; 0x1000];
    code3[0..4].copy_from_slice(&ret.to_le_bytes());
    mem.map(0x3000, code3, Permissions::READ_EXECUTE).unwrap();

    // Set X1 to target 0x2000
    cpu.x[1] = 0x2000;
    // Step BLR X1
    cpu.step(&mut mem).unwrap();
    // PC must be 0x2000 and LR (X30) must be 0x1004
    assert_eq!(cpu.pc, 0x2000);
    assert_eq!(cpu.x[30], 0x1004);

    // Set X2 to target 0x3000
    cpu.x[2] = 0x3000;
    // Step BR X2
    cpu.step(&mut mem).unwrap();
    // PC must be 0x3000 and X30 must be preserved as 0x1004
    assert_eq!(cpu.pc, 0x3000);
    assert_eq!(cpu.x[30], 0x1004);

    // Step RET (returns to X30 = 0x1004)
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.pc, 0x1004);
}

#[test]
fn test_test_bit_branch_tbz_tbnz() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // TBZ X0, #3, +8 (0x36180040)
    let tbz = 0x36180040u32;
    // TBNZ X0, #3, +8 (0x37180040)
    let tbnz = 0x37180040u32;

    let mut code = vec![0u8; 0x1000];
    code[0..4].copy_from_slice(&tbz.to_le_bytes());
    code[4..8].copy_from_slice(&tbnz.to_le_bytes());
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    // X0 has bit 3 cleared -> TBZ branches (+8: 0x1000 + 8 = 0x1008)
    cpu.x[0] = 0b0000;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.pc, 0x1008);

    // Reset PC to 0x1000 with bit 3 set -> TBZ falls through to 0x1004
    cpu.pc = 0x1000;
    cpu.x[0] = 0b1000;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.pc, 0x1004);

    // At 0x1004: TBNZ with bit 3 set -> branches (+8: 0x1004 + 8 = 0x100c)
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.pc, 0x100c);
}

#[test]
fn test_conditional_select_and_arithmetic() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // CSEL X0, X1, X2, EQ (0x9a820020)
    let csel_eq = 0x9a820020u32;
    // MUL X3, X1, X2 (0x9b027c23: MADD X3, X1, X2, XZR)
    let mul = 0x9b027c23u32;
    // UDIV X4, X1, X2 (0x9ac20824)
    let udiv = 0x9ac20824u32;
    // AND X5, X1, #0xffffffff (0x92407c25)
    let and_imm = 0x92407c25u32;

    let mut code = vec![0u8; 0x1000];
    code[0..4].copy_from_slice(&csel_eq.to_le_bytes());
    code[4..8].copy_from_slice(&mul.to_le_bytes());
    code[8..12].copy_from_slice(&udiv.to_le_bytes());
    code[12..16].copy_from_slice(&and_imm.to_le_bytes());
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    cpu.x[1] = 100;
    cpu.x[2] = 20;
    cpu.nzcv.zero = true; // EQ condition holds

    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 100); // EQ holds -> X0 = X1 = 100

    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[3], 2000); // 100 * 20 = 2000

    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[4], 5); // 100 / 20 = 5

    cpu.x[1] = 0x1234_5678_9abc_def0;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[5], 0x9abc_def0); // masked to 32 bits
}

#[test]
fn test_byte_halfword_unscaled_and_register_loads_stores() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // STRB W2, [X1, #2] (0x39000822)
    let strb = 0x39000822u32;
    // LDRB W0, [X1, #2] (0x39400820)
    let ldrb = 0x39400820u32;
    // LDRSB X3, [X1, #2] (0x39800823)
    let ldrsb = 0x39800823u32;
    // STRH W4, [X1, #4] (0x79000824)
    let strh = 0x79000824u32;
    // LDRH W5, [X1, #4] (0x79400825)
    let ldrh = 0x79400825u32;
    // LDUR X6, [X1, #8] (0xf8408026)
    let ldur = 0xf8408026u32;

    let mut code = vec![0u8; 0x1000];
    code[0..4].copy_from_slice(&strb.to_le_bytes());
    code[4..8].copy_from_slice(&ldrb.to_le_bytes());
    code[8..12].copy_from_slice(&ldrsb.to_le_bytes());
    code[12..16].copy_from_slice(&strh.to_le_bytes());
    code[16..20].copy_from_slice(&ldrh.to_le_bytes());
    code[20..24].copy_from_slice(&ldur.to_le_bytes());
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();
    mem.map(0x2000, vec![0u8; 0x1000], Permissions::READ_WRITE).unwrap();

    cpu.x[1] = 0x2000;
    cpu.x[2] = 0xfe; // byte: -2 signed
    cpu.x[4] = 0x1234; // halfword
    mem.write_u64(0x2008, 0xbeef_cafe_1234_5678).unwrap();

    // Step STRB
    cpu.step(&mut mem).unwrap();
    assert_eq!(mem.read_u8(0x2002).unwrap(), 0xfe);

    // Step LDRB
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 0xfe);

    // Step LDRSB (sign extended to 64-bit)
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[3], 0xffff_ffff_ffff_fffe);

    // Step STRH
    cpu.step(&mut mem).unwrap();
    assert_eq!(mem.read_u16(0x2004).unwrap(), 0x1234);

    // Step LDRH
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[5], 0x1234);

    // Step LDUR
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[6], 0xbeef_cafe_1234_5678);
}

#[test]
fn test_data_processing_1src_and_retail_rbit() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // Instructions:
    // 0: rbit x14, x13 (0xdac001ae) - the exact instruction from Pokémon Legends: Z-A fault at 0x1017d0!
    // 4: rbit w14, w13 (0x5ac001ae)
    // 8: rev16 w14, w13 (0x5ac005ae)
    // 12: rev16 x14, x13 (0xdac005ae)
    // 16: rev w14, w13 (0x5ac009ae)
    // 20: rev32 x14, x13 (0xdac009ae)
    // 24: rev x14, x13 (0xdac00dae)
    // 28: clz w14, w13 (0x5ac011ae)
    // 32: clz x14, x13 (0xdac011ae)
    // 36: cls w14, w13 (0x5ac015ae)
    // 40: cls x14, x13 (0xdac015ae)
    let insns: [u32; 11] = [
        0xdac001ae,
        0x5ac001ae,
        0x5ac005ae,
        0xdac005ae,
        0x5ac009ae,
        0xdac009ae,
        0xdac00dae,
        0x5ac011ae,
        0xdac011ae,
        0x5ac015ae,
        0xdac015ae,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    // 0: RBIT X14, X13
    cpu.x[13] = 0x1234_5678_9abc_def0;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x1234_5678_9abc_def0_u64.reverse_bits());

    // 4: RBIT W14, W13
    cpu.x[13] = 0x1234_5678;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x1e6a2c48);

    // 8: REV16 W14, W13
    cpu.x[13] = 0x1234_5678;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x3412_7856);

    // 12: REV16 X14, X13
    cpu.x[13] = 0x1122_3344_5566_7788;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x2211_4433_6655_8877);

    // 16: REV W14, W13
    cpu.x[13] = 0x1234_5678;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x7856_3412);

    // 20: REV32 X14, X13
    cpu.x[13] = 0x1122_3344_5566_7788;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x4433_2211_8877_6655);

    // 24: REV X14, X13
    cpu.x[13] = 0x1122_3344_5566_7788;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x8877_6655_4433_2211);

    // 28: CLZ W14, W13
    cpu.x[13] = 0x0001_0000;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 15);

    // 32: CLZ X14, X13
    cpu.x[13] = 0x0000_0001_0000_0000;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 31);

    // 36: CLS W14, W13
    cpu.x[13] = 0x8000_0000; // Sign is 1, next bit is 0 -> 0 sign bits
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0);

    // 40: CLS X14, X13
    cpu.x[13] = 0; // All 0s -> 63 leading sign bits
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 63);
}

#[test]
fn test_hints_and_barriers_nop() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // paciasp, autiasp, clrex, dmb ish, dsb ish, isb, yield
    let insns: [u32; 7] = [
        0xd503233f, // paciasp
        0xd50323bf, // autiasp
        0xd5033f5f, // clrex
        0xd5033bbf, // dmb ish
        0xd5033b9f, // dsb ish
        0xd5033fdf, // isb
        0xd503203f, // yield
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    cpu.x[30] = 0x1234_5678; // LR
    for i in 0..insns.len() {
        assert_eq!(cpu.step(&mut mem).unwrap(), StepEvent::Continue);
        assert_eq!(cpu.pc, 0x1000 + (i as u64 + 1) * 4);
    }
    // LR unchanged
    assert_eq!(cpu.x[30], 0x1234_5678);
}

#[test]
fn test_bitfield_bfm_bfi_bfxil_and_extract() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // 0: bfi w14, w13, #4, #8 (0x331c1dae)
    // 4: bfxil w14, w13, #4, #8 (0x33042dae)
    // 8: extr w14, w13, w12, #5 (0x138c15ae)
    // 12: extr x14, x13, x12, #5 (0x93cc15ae)
    let insns: [u32; 4] = [
        0x331c1dae,
        0x33042dae,
        0x138c15ae,
        0x93cc15ae,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    // BFI: insert 8 bits from w13 into w14 at pos 4
    cpu.x[14] = 0x1111_1111;
    cpu.x[13] = 0x22;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x1111_1221);

    // BFXIL: extract 8 bits from w13 at pos 4, insert into w14 at pos 0
    cpu.x[14] = 0x1111_1111;
    cpu.x[13] = 0x220;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0x1111_1122);

    // EXTR 32-bit: concat(w13, w12) >> 5
    // w13 = 0x12345678, w12 = 0x9abcdef0
    cpu.x[13] = 0x1234_5678;
    cpu.x[12] = 0x9abc_def0;
    cpu.step(&mut mem).unwrap();
    let expected32 = (((0x1234_5678_u64 << 32) | 0x9abc_def0_u64) >> 5) as u32 as u64;
    assert_eq!(cpu.x[14], expected32);

    // EXTR 64-bit: concat(x13, x12) >> 5
    cpu.x[13] = 0x1122_3344_5566_7788;
    cpu.x[12] = 0x99aa_bbcc_ddee_ff00;
    cpu.step(&mut mem).unwrap();
    let expected64 = (((0x1122_3344_5566_7788_u128 << 64) | 0x99aa_bbcc_ddee_ff00_u128) >> 5) as u64;
    assert_eq!(cpu.x[14], expected64);
}

#[test]
fn test_add_sub_carry_and_conditional_compare() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // 0: adcs w14, w13, w12 (0x3a0c01ae)
    // 4: sbcs w14, w13, w12 (0x7a0c01ae)
    // 8: ccmp w13, w12, #4, eq (0x7a4c01a4)
    // 12: ccmp w13, #5, #4, ne (0x7a4519a4)
    // 16: ccmn w13, w12, #4, eq (0x3a4c01a4)
    let insns: [u32; 5] = [
        0x3a0c01ae,
        0x7a0c01ae,
        0x7a4c01a4,
        0x7a4519a4,
        0x3a4c01a4,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    // ADCS: w13 + w12 + carry (carry=1)
    cpu.x[13] = 0xffff_ffff;
    cpu.x[12] = 0;
    cpu.nzcv.carry = true;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 0);
    assert!(cpu.nzcv.zero);
    assert!(cpu.nzcv.carry); // overflowed 32-bit

    // SBCS: w13 + NOT(w12) + carry
    // 5 - 3 with C=1 -> 2
    cpu.x[13] = 5;
    cpu.x[12] = 3;
    cpu.nzcv.carry = true;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 2);
    assert!(!cpu.nzcv.zero);
    assert!(cpu.nzcv.carry); // no borrow

    // CCMP w13, w12, #4, eq:
    // If eq (Z=1): compare w13 with w12.
    // Currently Z=0 (from previous sbcs), so eq is false!
    // Should set NZCV to default #4 (Z=1, N=0, C=0, V=0).
    cpu.step(&mut mem).unwrap();
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: false,
            zero: true,
            carry: false,
            overflow: false,
        }
    );

    // CCMP w13, #5, #4, ne:
    // Now Z=1, so ne is false!
    // Should set NZCV to #4 again.
    cpu.step(&mut mem).unwrap();
    assert!(cpu.nzcv.zero);

    // Now set Z=1, test CCMP when condition holds:
    // CCMN w13, w12, #4, eq:
    // Z=1 so eq is true!
    // Performs w13 + w12: 5 + 3 = 8 -> Z=0, N=0, C=0, V=0.
    cpu.step(&mut mem).unwrap();
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: false,
            zero: false,
            carry: false,
            overflow: false,
        }
    );
}

#[test]
fn test_wide_multiplies() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // 0: smaddl x14, w13, w12, x11 (0x9b2c2dae)
    // 4: smsubl x14, w13, w12, x11 (0x9b2cadae)
    // 8: umaddl x14, w13, w12, x11 (0x9bac2dae)
    // 12: smulh x14, x13, x12 (0x9b4c7dae)
    // 16: umulh x14, x13, x12 (0x9bcc7dae)
    let insns: [u32; 5] = [
        0x9b2c2dae,
        0x9b2cadae,
        0x9bac2dae,
        0x9b4c7dae,
        0x9bcc7dae,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    // SMADDL: x11 + (signed(w13) * signed(w12))
    // x11 = 10, w13 = -2 (0xfffffffe), w12 = 3 -> 10 + (-6) = 4
    cpu.x[11] = 10;
    cpu.x[13] = 0xffff_fffe;
    cpu.x[12] = 3;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 4);

    // SMSUBL: x11 - (signed(w13) * signed(w12))
    // 10 - (-6) = 16
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[14], 16);

    // UMADDL: x11 + (unsigned(w13) * unsigned(w12))
    // x11 = 10, w13 = 0xffff_fffe, w12 = 3
    cpu.step(&mut mem).unwrap();
    let expected = 10u64.wrapping_add((0xffff_fffe_u64) * 3);
    assert_eq!(cpu.x[14], expected);

    // SMULH: high 64 bits of signed x13 * x12
    cpu.x[13] = 0x4000_0000_0000_0000;
    cpu.x[12] = 2;
    cpu.step(&mut mem).unwrap();
    // 0x4000... * 2 = 0x8000_0000_0000_0000 -> sign bit 1 in 128-bit = negative, high 64 bits is 0
    let smul = (cpu.x[13] as i64 as i128) * (cpu.x[12] as i64 as i128);
    assert_eq!(cpu.x[14], (smul >> 64) as u64);

    // UMULH: high 64 bits of unsigned x13 * x12
    cpu.x[13] = 0x8000_0000_0000_0000;
    cpu.x[12] = 2;
    cpu.step(&mut mem).unwrap();
    // 0x8000... * 2 = 0x1_0000_0000_0000_0000 -> high 64 bits is 1
    assert_eq!(cpu.x[14], 1);
}

#[test]
fn test_conditional_select_variants_and_cset_retail() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // 0: cset w8, eq (0x1a9f17e8) - Pokémon Legends: Z-A retail opcode
    // 4: cset x9, ne (0x9a9f07e9)
    // 8: csinc w10, w1, w2, eq (0x1a82042a)
    // 12: csinv w12, w1, w2, ne (0x5a82102c)
    // 16: csneg x13, x1, x2, eq (0xda82042d)
    let insns: [u32; 5] = [
        0x1a9f17e8,
        0x9a9f07e9,
        0x1a82042a,
        0x5a82102c,
        0xda82042d,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    // Case 1: zero = true (EQ holds, NE false)
    cpu.nzcv.zero = true;
    cpu.x[1] = 10;
    cpu.x[2] = 20;

    // cset w8, eq -> EQ holds -> w8 = 1
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[8], 1);

    // cset x9, ne -> NE does not hold -> x9 = 0
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[9], 0);

    // csinc w10, w1, w2, eq -> EQ holds -> w10 = w1 = 10
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[10], 10);

    // csinv w12, w1, w2, ne -> NE does not hold -> w12 = !w2 = !20 (in 32-bit: 0xffff_ffeb)
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[12], (!20u32) as u64);

    // csneg x13, x1, x2, eq -> EQ holds -> x13 = x1 = 10
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[13], 10);

    // Case 2: zero = false (EQ false, NE holds)
    cpu.pc = 0x1000;
    cpu.nzcv.zero = false;
    cpu.x[1] = 10;
    cpu.x[2] = 20;

    // cset w8, eq -> EQ false -> w8 = 0
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[8], 0);

    // cset x9, ne -> NE holds -> x9 = 1
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[9], 1);

    // csinc w10, w1, w2, eq -> EQ false -> w10 = w2 + 1 = 21
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[10], 21);

    // csinv w12, w1, w2, ne -> NE holds -> w12 = w1 = 10
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[12], 10);

    // csneg x13, x1, x2, eq -> EQ false -> x13 = -x2 = -20
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[13], 0u64.wrapping_sub(20));
}

#[test]
fn test_add_sub_extended_register() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // 0: add x0, x1, w2, uxtw #3 (0x8b224c20)
    // 4: add x3, x1, w2, sxtw (0x8b22c023)
    // 8: cmp x1, w2, uxtw (0xeb22403f: subs xzr, x1, w2, uxtw)
    // 12: add sp, sp, x0 (0x8b2063ff)
    let insns: [u32; 4] = [
        0x8b224c20,
        0x8b22c023,
        0xeb22403f,
        0x8b2063ff,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();

    // ADD X0, X1, W2, UXTW #3:
    // x1 = 100, w2 = 5 -> uxtw(5) << 3 = 40 -> 100 + 40 = 140
    cpu.x[1] = 100;
    cpu.x[2] = 5;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 140);

    // ADD X3, X1, W2, SXTW:
    // x1 = 100, w2 = 0xffff_fffe (-2) -> sxtw(-2) = -2 -> 100 + (-2) = 98
    cpu.x[2] = 0xffff_fffe;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[3], 98);

    // CMP X1, W2, UXTW:
    // x1 = 100, w2 = 100 -> subs xzr, 100, 100 -> result 0, zero=true
    cpu.x[2] = 100;
    cpu.step(&mut mem).unwrap();
    assert!(cpu.nzcv.zero);
    assert!(!cpu.nzcv.negative);

    // ADD SP, SP, X0:
    // sp = 0x8000, x0 = 140 -> sp = 0x808c
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.sp, 0x8000 + 140);

    // Reserved encoding rejection: amount > 4
    let mut bad_mem = AddressSpace::default();
    let bad_amount = 0x8b225420u32; // imm3 = 5 > 4
    bad_mem.map(0x2000, bad_amount.to_le_bytes().to_vec(), Permissions::READ_EXECUTE).unwrap();
    let mut bad_cpu = Cpu::new(0x2000, 0x8000);
    assert!(bad_cpu.step(&mut bad_mem).is_err());
}

#[test]
fn test_exclusive_load_store_variants_and_ldar_stlr() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // 0: ldar w0, [x1] (0x88dffc20)
    // 4: stlr w0, [x1] (0x889ffc20)
    // 8: ldaxr w0, [x1] (0x885ffc20)
    // 12: stlxr w2, w0, [x1] (0x8802fc20)
    // 16: ldarb w3, [x1] (0x08dffc23)
    // 20: stlrb w3, [x1] (0x089ffc23)
    let insns: [u32; 6] = [
        0x88dffc20,
        0x889ffc20,
        0x885ffc20,
        0x8802fc20,
        0x08dffc23,
        0x089ffc23,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();
    mem.map(0x2000, vec![0u8; 0x1000], Permissions::READ_WRITE).unwrap();

    mem.write_u32(0x2000, 0x1234_5678).unwrap();
    cpu.x[1] = 0x2000;

    // LDAR
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 0x1234_5678);

    // STLR (write 0x4321 to [x1])
    cpu.x[0] = 0x4321;
    cpu.step(&mut mem).unwrap();
    assert_eq!(mem.read_u32(0x2000).unwrap(), 0x4321);

    // LDAXR
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 0x4321);

    // STLXR (status in w2)
    cpu.x[0] = 0x9999;
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[2], 0); // status success
    assert_eq!(mem.read_u32(0x2000).unwrap(), 0x9999);

    // LDARB
    mem.write_u8(0x2000, 0xef).unwrap();
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[3], 0xef);

    // STLRB
    cpu.x[3] = 0x42;
    cpu.step(&mut mem).unwrap();
    assert_eq!(mem.read_u8(0x2000).unwrap(), 0x42);
}

#[test]
fn test_ldpsw_signed_pair_loading() {
    let mut mem = AddressSpace::default();
    let mut cpu = Cpu::new(0x1000, 0x8000);

    // 0: ldpsw x0, x1, [x2] (0x69400440)
    // 4: ldpsw x3, x4, [x2, #8] (0x69411043)
    let insns: [u32; 2] = [
        0x69400440,
        0x69411043,
    ];

    let mut code = vec![0u8; 0x1000];
    for (i, &w) in insns.iter().enumerate() {
        code[i * 4..(i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }
    mem.map(0x1000, code, Permissions::READ_EXECUTE).unwrap();
    mem.map(0x2000, vec![0u8; 0x1000], Permissions::READ_WRITE).unwrap();

    // Write [ -2 (0xfffffffe), 5, -10 (0xfffffff6), 20 ]
    mem.write_u32(0x2000, 0xffff_fffe).unwrap();
    mem.write_u32(0x2004, 5).unwrap();
    mem.write_u32(0x2008, 0xffff_fff6).unwrap();
    mem.write_u32(0x200c, 20).unwrap();

    cpu.x[2] = 0x2000;

    // LDPSW x0, x1, [x2]
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[0], 0xffff_ffff_ffff_fffe); // sign-extended to 64 bits
    assert_eq!(cpu.x[1], 5);

    // LDPSW x3, x4, [x2, #8]
    cpu.step(&mut mem).unwrap();
    assert_eq!(cpu.x[3], 0xffff_ffff_ffff_fff6); // sign-extended to 64 bits
    assert_eq!(cpu.x[4], 20);
}

