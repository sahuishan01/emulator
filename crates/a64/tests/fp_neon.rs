use emulator_a64::{Cpu, Nzcv, StepEvent};
use emulator_memory::{AddressSpace, Permissions};

fn setup_cpu_with_code(code: &[u32]) -> (Cpu, AddressSpace) {
    let mut memory = AddressSpace::default();
    let code_base = 0x10_0000u64;
    let stack_base = 0x8000_0000u64;
    let stack_size = 0x1_0000usize;

    let mut code_bytes = Vec::new();
    for &instruction in code {
        code_bytes.extend_from_slice(&instruction.to_le_bytes());
    }
    // Pad to 4096
    while code_bytes.len() % 4096 != 0 {
        code_bytes.extend_from_slice(&0xd503201fu32.to_le_bytes()); // NOP
    }

    memory
        .map(code_base, code_bytes, Permissions::READ_EXECUTE)
        .expect("code map");
    memory
        .map(0x2000, vec![0; 4096], Permissions::READ_WRITE)
        .expect("data map");
    memory
        .map(stack_base, vec![0; stack_size], Permissions::READ_WRITE)
        .expect("stack map");

    let cpu = Cpu::new(code_base, stack_base + stack_size as u64);
    (cpu, memory)
}

#[test]
fn test_scalar_f32_arithmetic() {
    // Test FADD, FSUB, FMUL, FDIV, FABS, FNEG on single-precision floats (S registers)
    // S0 = 12.5, S1 = 2.5
    // FADD S2, S0, S1 -> 15.0
    // FSUB S3, S0, S1 -> 10.0
    // FMUL S4, S0, S1 -> 31.25
    // FDIV S5, S0, S1 -> 5.0
    // FNEG S6, S0 -> -12.5
    // FABS S7, S6 -> 12.5
    // SVC #0x42

    let mut cpu = Cpu::new(0x10_0000, 0x8001_0000);
    cpu.write_v_f32(0, 12.5);
    cpu.write_v_f32(1, 2.5);

    let code = [
        // FADD S2, S0, S1: 0x1e202800 | (1 << 16) | (0 << 5) | 2
        0x1e212802,
        // FSUB S3, S0, S1: 0x1e203800 | (1 << 16) | (0 << 5) | 3
        0x1e213803,
        // FMUL S4, S0, S1: 0x1e200800 | (1 << 16) | (0 << 5) | 4
        0x1e210804,
        // FDIV S5, S0, S1: 0x1e201800 | (1 << 16) | (0 << 5) | 5
        0x1e211805,
        // FNEG S6, S0: 0x1e204000 with op=2 (0x1e214000) | (0 << 5) | 6
        0x1e214006,
        // FABS S7, S6: op=1 (0x1e20c000) | (6 << 5) | 7
        0x1e20c0c7,
        // SVC #0x42
        0xd4000841,
    ];

    let (_, mut memory) = setup_cpu_with_code(&code);
    for _ in 0..10 {
        if let StepEvent::SupervisorCall { immediate } = cpu.step(&mut memory).unwrap() {
            assert_eq!(immediate, 0x42);
            break;
        }
    }

    assert_eq!(cpu.read_v_f32(2), 15.0);
    assert_eq!(cpu.read_v_f32(3), 10.0);
    assert_eq!(cpu.read_v_f32(4), 31.25);
    assert_eq!(cpu.read_v_f32(5), 5.0);
    assert_eq!(cpu.read_v_f32(6), -12.5);
    assert_eq!(cpu.read_v_f32(7), 12.5);
}

#[test]
fn test_scalar_f64_arithmetic_and_fma() {
    // D0 = 10.0, D1 = 3.0, D2 = 5.0
    // FMADD D3, D0, D1, D2 -> D2 + D0 * D1 = 5.0 + 30.0 = 35.0
    // FMSUB D4, D0, D1, D2 -> D2 - D0 * D1 = 5.0 - 30.0 = -25.0
    // FSQRT D5, D0 -> sqrt(10.0)
    let mut cpu = Cpu::new(0x10_0000, 0x8001_0000);
    cpu.write_v_f64(0, 10.0);
    cpu.write_v_f64(1, 3.0);
    cpu.write_v_f64(2, 5.0);

    let code = [
        // FMADD D3, D0, D1, D2: type=1 (bit 22), sub=0, neg=0 -> 0x1f400000 | (1 << 16) | (2 << 10) | (0 << 5) | 3
        0x1f410803,
        // FMSUB D4, D0, D1, D2: sub=1 (bit 21) -> 0x1f610804
        0x1f610804,
        // FSQRT D5, D0: 1-source double (0x1e61c000) | (0 << 5) | 5
        0x1e61c005,
        // SVC #1
        0xd4000021,
    ];

    let (_, mut memory) = setup_cpu_with_code(&code);
    for _ in 0..10 {
        if let StepEvent::SupervisorCall { immediate } = cpu.step(&mut memory).unwrap() {
            assert_eq!(immediate, 1);
            break;
        }
    }

    assert_eq!(cpu.read_v_f64(3), 35.0);
    assert_eq!(cpu.read_v_f64(4), -25.0);
    assert!((cpu.read_v_f64(5) - 10.0f64.sqrt()).abs() < 1e-12);
}

#[test]
fn test_fcmp_and_nzcv_flags() {
    let mut cpu = Cpu::new(0x10_0000, 0x8001_0000);

    // FCMP S0, S1:
    // Case 1: 5.0 == 5.0 -> N=0, Z=1, C=1, V=0
    cpu.write_v_f32(0, 5.0);
    cpu.write_v_f32(1, 5.0);
    let code1 = [0x1e212000]; // FCMP S0, S1
    let (_, mut memory1) = setup_cpu_with_code(&code1);
    cpu.step(&mut memory1).unwrap();
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: false,
            zero: true,
            carry: true,
            overflow: false
        }
    );

    // Case 2: 3.0 < 7.0 -> N=1, Z=0, C=0, V=0
    cpu.write_v_f32(0, 3.0);
    cpu.write_v_f32(1, 7.0);
    cpu.pc = 0x10_0000;
    cpu.step(&mut memory1).unwrap();
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: true,
            zero: false,
            carry: false,
            overflow: false
        }
    );

    // Case 3: 9.0 > 2.0 -> N=0, Z=0, C=1, V=0
    cpu.write_v_f32(0, 9.0);
    cpu.write_v_f32(1, 2.0);
    cpu.pc = 0x10_0000;
    cpu.step(&mut memory1).unwrap();
    assert_eq!(
        cpu.nzcv,
        Nzcv {
            negative: false,
            zero: false,
            carry: true,
            overflow: false
        }
    );

    // Case 4: NaN unordered -> N=0, Z=0, C=1, V=1
    cpu.write_v_f32(0, f32::NAN);
    cpu.write_v_f32(1, 1.0);
    cpu.pc = 0x10_0000;
    cpu.step(&mut memory1).unwrap();
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
fn test_conversions_and_fmov() {
    let mut cpu = Cpu::new(0x10_0000, 0x8001_0000);
    // W0 = 42
    cpu.x[0] = 42;

    let code = [
        // SCVTF S0, W0: 0x1e220000 | (0 << 5) | 0
        0x1e220000,
        // FCVT D1, S0 (single to double): 0x1e22c000 | (0 << 5) | 1
        0x1e22c001,
        // FCVTZS W2, D1 (double to signed int): 0x1e780000 | (1 << 5) | 2
        0x1e780022,
        // FMOV W3, S0: 0x1e270000 | (0 << 5) | 3
        0x1e270003,
        // SVC #99
        0xd4000c61,
    ];

    let (_, mut memory) = setup_cpu_with_code(&code);
    for _ in 0..10 {
        if let StepEvent::SupervisorCall { immediate } = cpu.step(&mut memory).unwrap() {
            assert_eq!(immediate, 99);
            break;
        }
    }

    assert_eq!(cpu.read_v_f32(0), 42.0);
    assert_eq!(cpu.read_v_f64(1), 42.0);
    assert_eq!(cpu.x[2], 42);
    assert_eq!(cpu.x[3], 42.0f32.to_bits() as u64);
}

#[test]
fn test_simd_fp_load_store_and_vector_ops() {
    let (mut cpu, _memory) = setup_cpu_with_code(&[0xd503201f]); // NOP
    let data_addr = 0x2000u64;

    // Store 128-bit Q0 to memory, load into Q1
    cpu.write_v_u128(0, 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210u128);
    cpu.x[10] = data_addr;

    let code = [
        // STR Q0, [X10]: 0x3d800000 | (10 << 5) | 0
        0x3d800140,
        // LDR Q1, [X10]: 0x3dc00000 | (10 << 5) | 1
        0x3dc00141,
        // EOR V2.16B, V0.16B, V1.16B -> should be all zeros: 0x6e201c00 | (1 << 16) | (0 << 5) | 2
        0x6e211c02,
        // ORR V3.16B, V0.16B, V1.16B -> should equal Q0: 0x4ea01c00 | (1 << 16) | (0 << 5) | 3
        0x4ea11c03,
        // SVC #7
        0xd40000e1,
    ];

    let (_, mut memory) = setup_cpu_with_code(&code);
    cpu.x[10] = data_addr;
    cpu.pc = 0x10_0000;
    for _ in 0..10 {
        if let StepEvent::SupervisorCall { immediate } = cpu.step(&mut memory).unwrap() {
            assert_eq!(immediate, 7);
            break;
        }
    }

    assert_eq!(cpu.read_v_u128(1), 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210u128);
    assert_eq!(cpu.read_v_u128(2), 0);
    assert_eq!(cpu.read_v_u128(3), 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210u128);
}
