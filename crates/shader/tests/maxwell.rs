use emulator_shader::census::SELF_BRANCH;
use emulator_shader::maxwell::{decode_word, list, Instr, MufuOp, Pred, Source, PT};

fn instr(word: u64) -> Instr {
    decode_word(word).expect("decodes").instr
}

#[test]
fn guard_and_control_flow() {
    let exit = decode_word(0xe30000000007000f).unwrap();
    assert!(exit.guard.is_always());
    assert_eq!(exit.instr, Instr::Exit { cc: 0xf });
    // @!P0 EXIT
    assert_eq!(
        decode_word(0xe30000000008000f).unwrap().guard,
        Pred {
            index: 0,
            negate: true
        }
    );
    assert_eq!(
        instr(SELF_BRANCH),
        Instr::Bra {
            cc: 0xf,
            offset: -8
        }
    );
    assert_eq!(instr(0xe29000000400_0000), Instr::Ssy { offset: 0x40 });
    assert_eq!(instr(0xf0f800000007000f), Instr::Sync { cc: 0xf });
    assert_eq!(instr(0x50b0000000070f00), Instr::Nop);
}

#[test]
fn alu_source_forms() {
    assert_eq!(
        instr(0x5c98078000370002),
        Instr::Mov {
            d: 2,
            b: Source::Reg(3),
            lanes: 0xf
        }
    );
    // c[0x2][0x10]
    assert_eq!(
        instr(0x4c98078800470002),
        Instr::Mov {
            d: 2,
            b: Source::Cbuf {
                bank: 2,
                offset: 0x10
            },
            lanes: 0xf
        }
    );
    // Negative immediate: sign bit 56.
    let Instr::Mov {
        b: Source::Imm20(imm),
        ..
    } = instr(0x399807ffffff0002)
    else {
        panic!()
    };
    assert_eq!(Source::imm_i32(imm), -1);
    // FADD imm 1.0 (0x3f800000 >> 12 = 0x3f800).
    let Instr::Fadd {
        b: Source::Imm20(imm),
        ..
    } = instr(0x3858003f80070102)
    else {
        panic!()
    };
    assert_eq!(Source::imm_f32_bits(imm), 0x3f80_0000);
    assert_eq!(
        instr(0x010012345670f003),
        Instr::Mov32i {
            d: 3,
            imm: 0x01234567,
            lanes: 0xf
        }
    );
}

#[test]
fn float_forms() {
    assert_eq!(
        instr(0x5980020000370102),
        Instr::Ffma {
            d: 2,
            a: 1,
            b: Source::Reg(3),
            c: Source::Reg(4),
            neg_b: false,
            neg_c: false,
            sat: false
        }
    );
    assert_eq!(
        instr(0x5080000000470102),
        Instr::Mufu {
            d: 2,
            a: 1,
            op: MufuOp::Rcp,
            neg: false,
            abs: false,
            sat: false
        }
    );
    let Instr::Fsetp { p, q, cmp, c, .. } = instr(0x5bb403800037010f) else {
        panic!()
    };
    assert_eq!((p, q, cmp, c.index), (1, 7, 4, PT));
    let Instr::Fmnmx { pred, .. } = instr(0x5c60038000370102) else {
        panic!()
    };
    assert_eq!(
        pred,
        Pred {
            index: PT,
            negate: false
        }
    );
}

#[test]
fn unknown_words_fail_and_listing_counts() {
    assert_eq!(decode_word(0x5c30000000070000), None); // FLO: not listed
    let mut code = Vec::new();
    for w in [
        0u64,
        0x5c98078000370002,
        0,
        0x5c30000000070000,
        0,
        SELF_BRANCH,
        0,
        0x5c98078000370002,
    ] {
        code.extend_from_slice(&w.to_le_bytes());
    }
    let l = list(&code);
    assert_eq!(l.instructions.len(), 2);
    assert_eq!(l.zero_words, 1);
    assert_eq!(l.unsupported, Some((24, 0x5c30000000070000)));
    assert_eq!(l.unsupported_count, 1);
}

#[test]
fn fmul32i_iadd_fcmp_fset() {
    let w = (0x1eu64 << 56) | (1 << 55) | (0x3f80_0000u64 << 20) | (7 << 16) | (2 << 8) | 1;
    assert_eq!(
        instr(w),
        Instr::Fmul32i {
            d: 1,
            a: 2,
            imm: 0x3f80_0000,
            sat: true
        }
    );
    // IADD R1, -R2, R3
    let w = (0x5c10u64 << 48) | (1 << 49) | (3 << 20) | (7 << 16) | (2 << 8) | 1;
    assert_eq!(
        instr(w),
        Instr::Iadd {
            d: 1,
            a: 2,
            b: Source::Reg(3),
            neg_a: true,
            neg_b: false,
            sat: false,
            x: false
        }
    );
    // FCMP.GE R0, R1, c[0x3][0x8], R4 (cmp 0xe in the opcode's low nibble)
    let w = (0x4baeu64 << 48) | (4 << 39) | (3 << 34) | (2 << 20) | (7 << 16) | (1 << 8);
    assert_eq!(
        instr(w),
        Instr::Fcmp {
            d: 0,
            a: 1,
            b: Source::Cbuf { bank: 3, offset: 8 },
            c: Source::Reg(4),
            cmp: 0xe
        }
    );
    // FSET.LT.FTZ.AND R5, R6, R7, PT (integer-mask result)
    let w = (0x5881u64 << 48) | (7 << 39) | (7 << 20) | (7 << 16) | (6 << 8) | 5;
    let Instr::Fset {
        d: 5,
        a: 6,
        b: Source::Reg(7),
        cmp: 1,
        bop: 0,
        c,
        float_result: false,
        ..
    } = instr(w)
    else {
        panic!("fset {:?}", instr(w));
    };
    assert_eq!(
        c,
        Pred {
            index: PT,
            negate: false
        }
    );
}

#[test]
fn undecoded_examples_are_distinct() {
    let mut code = vec![0u8; 32];
    code[8..16].copy_from_slice(&0x1234u64.to_le_bytes());
    code[16..24].copy_from_slice(&0x1234u64.to_le_bytes());
    code[24..32].copy_from_slice(&0x5678u64.to_le_bytes());
    let l = list(&code);
    assert_eq!(l.unsupported_count, 3);
    assert_eq!(l.undecoded_examples, vec![(8, 0x1234), (24, 0x5678)]);
}
