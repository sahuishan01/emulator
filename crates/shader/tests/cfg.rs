use emulator_shader::census::SELF_BRANCH;
use emulator_shader::cfg::{build, CfgError, Terminator};
use emulator_shader::maxwell::{list, Pred};

const EXIT: u64 = 0xe30000000007000f;
const SYNC: u64 = 0xf0f800000007000f;

fn ssy(offset: u32) -> u64 {
    0xe290_0000_0007_0000 | (offset as u64) << 20
}
fn bra(offset: i32, guard: u64) -> u64 {
    0xe240_0000_0000_000f | ((offset as u32 as u64) & 0xff_ffff) << 20 | guard << 16
}
/// Three instructions per 32-byte bundle; slot 0 is the (zero) control word.
fn code(words: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in words.chunks(3) {
        out.extend_from_slice(&0u64.to_le_bytes());
        for i in 0..3 {
            out.extend_from_slice(&chunk.get(i).copied().unwrap_or(0).to_le_bytes());
        }
    }
    out
}

#[test]
fn ssy_branch_sync_diamond() {
    // 8: SSY 48; 16: @P0 BRA 40; 24: SYNC; 40: SYNC; 48: EXIT; 56: self branch.
    let c = code(&[ssy(32), bra(16, 0), SYNC, SYNC, EXIT, SELF_BRANCH]);
    let cfg = build(&list(&c)).expect("cfg");
    assert_eq!(
        cfg.blocks.keys().copied().collect::<Vec<_>>(),
        [8, 24, 40, 48]
    );
    assert_eq!(
        cfg.blocks[&8].terminator,
        Terminator::Branch {
            guard: Pred {
                index: 0,
                negate: false
            },
            taken: 40,
            not_taken: 24
        }
    );
    assert_eq!(cfg.blocks[&24].terminator, Terminator::Jump(48));
    assert_eq!(cfg.blocks[&40].ssy_stack, vec![48]);
    assert_eq!(cfg.blocks[&48].terminator, Terminator::Exit);
    assert!(cfg.blocks[&48].ssy_stack.is_empty());
    assert_eq!(cfg.successors(8), vec![40, 24]);
}

#[test]
fn rejections() {
    assert_eq!(
        build(&list(&code(&[SYNC, EXIT]))),
        Err(CfgError::SyncWithoutSsy { at: 8 })
    );
    // Branch into a control-word slot.
    assert!(matches!(
        build(&list(&code(&[bra(0x10, 7), EXIT]))),
        Err(CfgError::BadTarget { at: 8, .. })
    ));
    // Guarded exit at the end with nothing after it.
    let guarded_exit = EXIT & !0xf_0000;
    assert_eq!(
        build(&list(&code(&[guarded_exit]))),
        Err(CfgError::FallsOffEnd { at: 8 })
    );
    // BRA -0x10 in slot 1 targets its own bundle start: a halt loop.
    let halt = build(&list(&code(&[bra(-16, 7)]))).expect("halt loop");
    assert_eq!(halt.blocks[&8].terminator, Terminator::Jump(8));
    // Unreached data after EXIT is ignored.
    assert!(build(&list(&code(&[EXIT, 0x1234]))).is_ok());
    assert!(matches!(
        build(&list(&code(&[0x1234, EXIT]))),
        Err(CfgError::Undecoded { offset: 8, .. })
    ));
    // Condition-code tested branch.
    assert_eq!(
        build(&list(&code(&[bra(0, 7) & !0xf | 0x2, EXIT]))),
        Err(CfgError::ConditionCode { at: 8, cc: 2 })
    );
}
