//! Offline decoder coverage audit.
//!
//! Reads `llvm-objdump -d --no-show-raw-insn=false` style lines from stdin
//! (`addr: wwwwwwww \t mnemonic operands`) and reports which mnemonics the
//! interpreter cannot decode, ordered by frequency.
use std::collections::BTreeMap;
use std::io::{self, BufRead};

fn main() {
    let mut total = 0_u64;
    let mut missing = 0_u64;
    let mut by_mnemonic: BTreeMap<String, (u64, u32)> = BTreeMap::new();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { continue };
        let Some((_, rest)) = line.split_once(':') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let Some(raw) = fields.next() else { continue };
        if raw.len() != 8 {
            continue;
        }
        let Ok(word) = u32::from_str_radix(raw, 16) else {
            continue;
        };
        let Some(mnemonic) = fields.next() else {
            continue;
        };
        if mnemonic == "<unknown>" || mnemonic == "udf" || mnemonic.starts_with('.') {
            continue;
        }
        total += 1;
        if !emulator_a64::is_decodable(word) {
            missing += 1;
            let entry = by_mnemonic.entry(mnemonic.to_owned()).or_insert((0, word));
            entry.0 += 1;
        }
    }
    let mut rows: Vec<_> = by_mnemonic.into_iter().collect();
    rows.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    println!("total={total} missing={missing} distinct={}", rows.len());
    for (mnemonic, (count, sample)) in rows {
        println!("{count:>8} {mnemonic:<12} e.g. {sample:#010x}");
    }
}
