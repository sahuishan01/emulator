use emulator_shader::{decode, emit, reference, Stage};
#[test]
fn portable_batch_passes_actual_semantics_and_rejections() {
    let cases = reference::run();
    assert_eq!(cases.len(), 25);
    for case in cases {
        assert!(case.passed, "{case:?}");
    }
}
#[test]
fn every_truncation_is_bounded_and_source_is_never_changed() {
    for (_, stage, code) in reference::FIXTURES {
        let original = code.to_vec();
        for end in 0..code.len() {
            let _ = decode(&code[..end], stage);
        }
        assert_eq!(code, original);
    }
}
#[test]
fn single_bit_mutations_fail_or_produce_owned_bounded_modules() {
    for (_, stage, code) in reference::FIXTURES {
        for offset in 0..code.len() {
            for bit in 0..8 {
                let mut mutant = code.to_vec();
                mutant[offset] ^= 1 << bit;
                if let Ok(p) = decode(&mutant, stage) {
                    let m = emit(&p).unwrap();
                    assert!(m.len() <= 2048);
                    let _ = p.evaluate([0; 4]);
                }
            }
        }
    }
}
#[test]
fn ssa_reads_capture_values_before_register_overwrite() {
    let (_, _, bytes) = reference::FIXTURES[1];
    let p = decode(bytes, Stage::Vertex).unwrap();
    assert_eq!(
        p.evaluate([2., 4., 0., 1.].map(f32::to_bits)).unwrap(),
        [1., 2., 0., 1.].map(f32::to_bits)
    );
    for (id, v) in p.values().iter().enumerate() {
        use emulator_shader::Op;
        match *v.op() {
            Op::BitsToFloat(a) | Op::FloatToBits(a) => assert!(a < id),
            Op::Add(a, b) | Op::Multiply(a, b) => assert!(a < id && b < id),
            _ => {}
        }
    }
}

#[test]
fn vertex_generic_export_uses_the_packed_vendor_header_layout() {
    // NVIDIA SPH Type1 table: common words, all IMAP fields, OMAP system A/B.
    // The generic OMAP follows 432 bits, not a whole number of 32-bit words.
    let fields = [32, 32, 32, 32, 32, 24, 8, 128, 16, 16, 40, 8, 24, 8];
    let generic_start: usize = fields.iter().sum();
    assert_eq!(generic_start, 432);
    let mut header = [0u8; 80];
    let mut set = |start: usize, width: usize, value: u32| {
        for bit in 0..width {
            if value & (1 << bit) != 0 {
                header[(start + bit) / 8] |= 1 << ((start + bit) % 8);
            }
        }
    };
    set(0, 32, 0x04020461);
    set(128, 32, 0x1000);
    set(192, 8, 0xff);
    set(generic_start - 4, 4, 15);
    set(generic_start, 4, 15);
    assert_eq!(
        u32::from_le_bytes(header[52..56].try_into().unwrap()),
        0x000ff000
    );
    let code = include_bytes!("../../../examples/shaders/texture.vert.bin");
    let mut program = header.to_vec();
    program.extend_from_slice(code);
    let p = emulator_shader::decode_program(&program, Stage::Vertex).unwrap();
    assert!(p.varying_outputs().is_some());
    let page = include_bytes!("../../../examples/gpu/texture-rgba-page.bin");
    assert_eq!(&page[0xa00..0xa50], &header);
    // Reproduce the initially misplaced word14 map and require rejection.
    program[54] = 0;
    program[56] = 15;
    assert_eq!(
        emulator_shader::decode_program(&program, Stage::Vertex),
        Err(emulator_shader::Error::Header)
    );
}
