//! A64 cryptographic extension: SHA1, SHA256 and AES instructions.
//!
//! Implemented from the architectural pseudocode semantics. The AES S-boxes are
//! derived at first use from the GF(2^8) multiplicative inverse and the affine map,
//! so no lookup tables are embedded. Unrecognised encodings return `None` before
//! touching CPU state.

use crate::Cpu;
use std::sync::OnceLock;

fn w(v: u128, index: usize) -> u32 {
    (v >> (32 * index)) as u32
}

fn words(v: u128) -> [u32; 4] {
    [w(v, 0), w(v, 1), w(v, 2), w(v, 3)]
}

fn join(x: [u32; 4]) -> u128 {
    x.iter()
        .enumerate()
        .fold(0, |acc, (i, &lane)| acc | (u128::from(lane) << (32 * i)))
}

fn choose(x: u32, y: u32, z: u32) -> u32 {
    ((y ^ z) & x) ^ z
}

fn parity(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}

fn majority(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | ((x | y) & z)
}

/// SHA1C / SHA1P / SHA1M: four rounds on hash state `x` with `e` = `y`.
pub(crate) fn sha1_rounds(kind: u32, x: u128, y: u32, wk: u128) -> u128 {
    let mut x = words(x);
    let mut y = y;
    for e in 0..4 {
        let t = match kind {
            0 => choose(x[1], x[2], x[3]),
            1 => parity(x[1], x[2], x[3]),
            _ => majority(x[1], x[2], x[3]),
        };
        y = y
            .wrapping_add(x[0].rotate_left(5))
            .wrapping_add(t)
            .wrapping_add(w(wk, e));
        x[1] = x[1].rotate_left(30);
        // Rotate the 160-bit Y:X left by 32 bits.
        let top = x[3];
        x = [y, x[0], x[1], x[2]];
        y = top;
    }
    join(x)
}

pub(crate) fn sha1su0(d: u128, n: u128, m: u128) -> u128 {
    ((n << 64) | (d >> 64)) ^ d ^ m
}

pub(crate) fn sha1su1(d: u128, n: u128) -> u128 {
    let t = words(d ^ (n >> 32));
    join([
        t[0].rotate_left(1),
        t[1].rotate_left(1),
        t[2].rotate_left(1),
        t[3].rotate_left(1) ^ t[0].rotate_left(2),
    ])
}

fn big_sigma0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

fn big_sigma1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

/// SHA256H (part1 = true, returns X) and SHA256H2 (returns Y).
pub(crate) fn sha256_hash(x: u128, y: u128, wk: u128, part1: bool) -> u128 {
    let (mut x, mut y) = (words(x), words(y));
    for e in 0..4 {
        let chs = choose(y[0], y[1], y[2]);
        let maj = majority(x[0], x[1], x[2]);
        let t = y[3]
            .wrapping_add(big_sigma1(y[0]))
            .wrapping_add(chs)
            .wrapping_add(w(wk, e));
        // X[3] becomes the new e, Y[3] the new a; the rotate moves them into place.
        x[3] = t.wrapping_add(x[3]);
        y[3] = t.wrapping_add(big_sigma0(x[0])).wrapping_add(maj);
        // Rotate the 256-bit Y:X left by 32 bits.
        let (x_top, y_top) = (x[3], y[3]);
        x = [y_top, x[0], x[1], x[2]];
        y = [x_top, y[0], y[1], y[2]];
    }
    join(if part1 { x } else { y })
}

pub(crate) fn sha256su0(d: u128, n: u128) -> u128 {
    let (d, n) = (words(d), words(n));
    let t = [d[1], d[2], d[3], n[0]];
    let mut out = [0; 4];
    for e in 0..4 {
        let elt = t[e];
        out[e] = (elt.rotate_right(7) ^ elt.rotate_right(18) ^ (elt >> 3)).wrapping_add(d[e]);
    }
    join(out)
}

pub(crate) fn sha256su1(d: u128, n: u128, m: u128) -> u128 {
    let (d, n, m) = (words(d), words(n), words(m));
    let t0 = [n[1], n[2], n[3], m[0]];
    let sigma1 = |x: u32| x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10);
    let mut out = [0; 4];
    for e in 0..2 {
        out[e] = sigma1(m[e + 2]).wrapping_add(d[e]).wrapping_add(t0[e]);
    }
    for e in 2..4 {
        out[e] = sigma1(out[e - 2]).wrapping_add(d[e]).wrapping_add(t0[e]);
    }
    join(out)
}

// ---------------------------------------------------------------------------
// AES
// ---------------------------------------------------------------------------

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0;
    while b != 0 {
        if b & 1 != 0 {
            product ^= a;
        }
        let carry = a & 0x80;
        a <<= 1;
        if carry != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    product
}

struct SBoxes {
    forward: [u8; 256],
    inverse: [u8; 256],
}

fn sboxes() -> &'static SBoxes {
    static TABLES: OnceLock<SBoxes> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut forward = [0u8; 256];
        let mut inverse = [0u8; 256];
        for value in 0..=255u8 {
            // Multiplicative inverse in GF(2^8); 0 maps to 0.
            let inv = if value == 0 {
                0
            } else {
                (1..=255u8).find(|&c| gf_mul(value, c) == 1).unwrap()
            };
            let affine = inv
                ^ inv.rotate_left(1)
                ^ inv.rotate_left(2)
                ^ inv.rotate_left(3)
                ^ inv.rotate_left(4)
                ^ 0x63;
            forward[usize::from(value)] = affine;
            inverse[usize::from(affine)] = value;
        }
        SBoxes { forward, inverse }
    })
}

fn bytes(v: u128) -> [u8; 16] {
    v.to_le_bytes()
}

/// State byte `i` is row `i % 4`, column `i / 4` (FIPS-197 column-major order).
pub(crate) fn aes_round(d: u128, n: u128, decrypt: bool) -> u128 {
    let state = bytes(d ^ n);
    let boxes = sboxes();
    let mut out = [0u8; 16];
    for column in 0..4 {
        for row in 0..4 {
            let source = if decrypt {
                (column + 4 - row) % 4
            } else {
                (column + row) % 4
            };
            let byte = state[row + 4 * source];
            out[row + 4 * column] = if decrypt {
                boxes.inverse[usize::from(byte)]
            } else {
                boxes.forward[usize::from(byte)]
            };
        }
    }
    u128::from_le_bytes(out)
}

pub(crate) fn aes_mix(n: u128, inverse: bool) -> u128 {
    let coefficients: [u8; 4] = if inverse {
        [14, 11, 13, 9]
    } else {
        [2, 3, 1, 1]
    };
    let state = bytes(n);
    let mut out = [0u8; 16];
    for column in 0..4 {
        let c = &state[4 * column..4 * column + 4];
        for row in 0..4 {
            out[row + 4 * column] = (0..4).fold(0, |acc, k| {
                acc ^ gf_mul(coefficients[(k + 4 - row) % 4], c[k])
            });
        }
    }
    u128::from_le_bytes(out)
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Executes a cryptographic-extension instruction, or returns `None` untouched.
pub(crate) fn execute(cpu: &mut Cpu, word: u32) -> Option<()> {
    let d = (word & 31) as usize;
    let n = ((word >> 5) & 31) as usize;
    let m = ((word >> 16) & 31) as usize;
    let opcode = (word >> 12) & 0x1f;
    if word & 0xffe0_8c00 == 0x5e00_0000 {
        // SHA three-register: opcode is 3 bits here.
        let (vd, vn, vm) = (cpu.v[d], cpu.v[n], cpu.v[m]);
        cpu.v[d] = match opcode & 7 {
            kind @ 0..=2 => sha1_rounds(kind, vd, vn as u32, vm),
            3 => sha1su0(vd, vn, vm),
            4 => sha256_hash(vd, vn, vm, true),
            5 => sha256_hash(vn, vd, vm, false),
            6 => sha256su1(vd, vn, vm),
            _ => return None,
        };
        return Some(());
    }
    if word & 0xfffe_0c00 == 0x5e28_0800 {
        let (vd, vn) = (cpu.v[d], cpu.v[n]);
        cpu.v[d] = match opcode {
            0 => u128::from((vn as u32).rotate_left(30)),
            1 => sha1su1(vd, vn),
            2 => sha256su0(vd, vn),
            _ => return None,
        };
        return Some(());
    }
    if word & 0xfffe_0c00 == 0x4e28_0800 {
        let (vd, vn) = (cpu.v[d], cpu.v[n]);
        cpu.v[d] = match opcode {
            4 => aes_round(vd, vn, false),
            5 => aes_round(vd, vn, true),
            6 => aes_mix(vn, false),
            7 => aes_mix(vn, true),
            _ => return None,
        };
        return Some(());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sbox_matches_fips197_samples() {
        let boxes = sboxes();
        assert_eq!(boxes.forward[0x00], 0x63);
        assert_eq!(boxes.forward[0x53], 0xed);
        assert_eq!(boxes.forward[0xff], 0x16);
        assert_eq!(boxes.inverse[0x63], 0x00);
    }

    #[test]
    fn mix_columns_round_trips() {
        let value = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210u128;
        assert_eq!(aes_mix(aes_mix(value, false), true), value);
        // FIPS-197 column example: db 13 53 45 -> 8e 4d a1 bc.
        let column =
            u128::from_le_bytes([0xdb, 0x13, 0x53, 0x45, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            &aes_mix(column, false).to_le_bytes()[..4],
            &[0x8e, 0x4d, 0xa1, 0xbc]
        );
    }
}
