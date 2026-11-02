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
