use emulator_shader::census::{identify, scan, SELF_BRANCH};

#[test]
fn identifies_forms_used_by_the_translator() {
    assert_eq!(identify(0x5c98078000070000), Some("MOV"));
    assert_eq!(identify(0x4c98078000070000), Some("MOV")); // cbuf form
    assert_eq!(identify(0x3898078000070000), Some("MOV")); // immediate
    assert_eq!(identify(0x5980000000070000), Some("FFMA"));
    assert_eq!(identify(0xe30000000007000f), Some("EXIT"));
    assert_eq!(identify(SELF_BRANCH), Some("BRA"));
    assert_eq!(identify(0x50b0000000070f00), Some("NOP"));
    assert_eq!(identify(0x0000000000000000), None);
}

#[test]
fn scan_stops_at_self_branch_and_counts_unknowns() {
    let mut code = Vec::new();
    for words in [
        [
            0u64,
            0x5c98078000070000,
            0x0000_0000_0000_0000,
            0xe30000000007000f,
        ],
        [0, SELF_BRANCH, 0x50b0000000070f00, 0x50b0000000070f00],
        [0, 0x5c98078000070000, 0, 0],
    ] {
        for w in words {
            code.extend_from_slice(&w.to_le_bytes());
        }
    }
    let census = scan(&code);
    assert!(census.terminated);
    assert_eq!(census.instructions, 6);
    assert_eq!(census.known["MOV"], 1);
    assert_eq!(census.known["NOP"], 2);
    assert_eq!(census.unknown[&0], 1);
}
