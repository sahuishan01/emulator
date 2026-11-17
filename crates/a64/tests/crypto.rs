//! Crypto-extension instructions executed through the CPU, checked against full
//! SHA-1, SHA-256 and AES-128 known-answer vectors. The host drives the algorithm
//! (message schedule constants, register moves); every SHA/AES step is a real
//! guest instruction word.

use emulator_a64::Cpu;
use emulator_memory::{AddressSpace, Permissions};

const CODE: u64 = 0x10_0000;

struct Guest {
    cpu: Cpu,
    memory: AddressSpace,
}

impl Guest {
    fn new() -> Self {
        Self {
            cpu: Cpu::new(CODE, 0x8000_0000),
            memory: AddressSpace::default(),
        }
    }

    /// Runs one instruction word at a fresh address (no stale decode reuse).
    fn exec(&mut self, word: u32) {
        let page = CODE + self.memory.mapped_bytes() as u64;
        let mut bytes = vec![0u8; 4096];
        bytes[..4].copy_from_slice(&word.to_le_bytes());
        self.memory
            .map(page, bytes, Permissions::READ_EXECUTE)
            .expect("code page");
        self.cpu.pc = page;
        self.cpu
            .step(&mut self.memory)
            .unwrap_or_else(|error| panic!("{word:#010x}: {error:?}"));
    }
}

fn three(base: u32, d: u32, n: u32, m: u32) -> u32 {
    base | (m << 16) | (n << 5) | d
}

fn two(base: u32, d: u32, n: u32) -> u32 {
    base | (n << 5) | d
}

fn lanes(x: [u32; 4]) -> u128 {
    x.iter()
        .enumerate()
        .fold(0, |acc, (i, &v)| acc | (u128::from(v) << (32 * i)))
}

fn lane(v: u128, i: usize) -> u32 {
    (v >> (32 * i)) as u32
}

fn add4(a: u128, b: u128) -> u128 {
    lanes([0, 1, 2, 3].map(|i| lane(a, i).wrapping_add(lane(b, i))))
}

fn padded_abc() -> [u32; 16] {
    let mut block = [0u8; 64];
    block[..3].copy_from_slice(b"abc");
    block[3] = 0x80;
    block[63] = 24;
    let mut out = [0u32; 16];
    for (i, word) in out.iter_mut().enumerate() {
        *word = u32::from_be_bytes(block[4 * i..4 * i + 4].try_into().unwrap());
    }
    out
}

#[test]
fn sha256_abc_through_guest_instructions() {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let initial = [
        0x6a09e667u32, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut guest = Guest::new();
    let block = padded_abc();
    let mut schedule: Vec<u128> = (0..4)
        .map(|i| lanes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]))
        .collect();
    // SHA256SU0 Vd, Vn then SHA256SU1 Vd, Vn, Vm extend the schedule.
    for i in 0..12 {
        guest.cpu.v[20] = schedule[i];
        guest.cpu.v[21] = schedule[i + 1];
        guest.cpu.v[22] = schedule[i + 2];
        guest.cpu.v[23] = schedule[i + 3];
        guest.exec(two(0x5e28_2800, 20, 21));
        guest.exec(three(0x5e00_6000, 20, 22, 23));
        schedule.push(guest.cpu.v[20]);
    }
    let abcd = lanes([initial[0], initial[1], initial[2], initial[3]]);
    let efgh = lanes([initial[4], initial[5], initial[6], initial[7]]);
    guest.cpu.v[0] = abcd;
    guest.cpu.v[1] = efgh;
    for (i, w) in schedule.iter().enumerate() {
        guest.cpu.v[16] = add4(*w, lanes([K[4 * i], K[4 * i + 1], K[4 * i + 2], K[4 * i + 3]]));
        guest.cpu.v[2] = guest.cpu.v[0];
        guest.exec(three(0x5e00_4000, 0, 1, 16)); // SHA256H q0, q1, v16.4s
        guest.exec(three(0x5e00_5000, 1, 2, 16)); // SHA256H2 q1, q2, v16.4s
    }
    let (abcd, efgh) = (add4(guest.cpu.v[0], abcd), add4(guest.cpu.v[1], efgh));
    let digest: Vec<u32> = (0..4).map(|i| lane(abcd, i)).chain((0..4).map(|i| lane(efgh, i))).collect();
    assert_eq!(
        digest,
        [
            0xba7816bf, 0x8f01cfea, 0x414140de, 0x5dae2223, 0xb00361a3, 0x96177a9c, 0xb410ff61,
            0xf20015ad
        ]
    );
}

#[test]
fn sha1_abc_through_guest_instructions() {
    let mut guest = Guest::new();
    let block = padded_abc();
    let mut schedule: Vec<u128> = (0..4)
        .map(|i| lanes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]))
        .collect();
    // SHA1SU0 Vd, Vn, Vm then SHA1SU1 Vd, Vn.
    for i in 0..16 {
        guest.cpu.v[20] = schedule[i];
        guest.cpu.v[21] = schedule[i + 1];
        guest.cpu.v[22] = schedule[i + 2];
        guest.cpu.v[23] = schedule[i + 3];
        guest.exec(three(0x5e00_3000, 20, 21, 22));
        guest.exec(two(0x5e28_1800, 20, 23));
        schedule.push(guest.cpu.v[20]);
    }
    let initial = lanes([0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476]);
    let initial_e = 0xc3d2e1f0u32;
    guest.cpu.v[0] = initial;
    let mut e = initial_e;
    for (group, w) in schedule.iter().enumerate() {
        let (k, opcode) = match group / 5 {
            0 => (0x5a827999u32, 0x5e00_0000), // SHA1C
            1 => (0x6ed9eba1, 0x5e00_1000),    // SHA1P
            2 => (0x8f1bbcdc, 0x5e00_2000),    // SHA1M
            _ => (0xca62c1d6, 0x5e00_1000),    // SHA1P
        };
        guest.cpu.v[16] = add4(*w, lanes([k; 4]));
        guest.cpu.v[1] = u128::from(e);
        guest.cpu.v[3] = guest.cpu.v[0];
        guest.exec(two(0x5e28_0800, 2, 3)); // SHA1H s2, s3: next E
        guest.exec(three(opcode, 0, 1, 16));
        e = guest.cpu.v[2] as u32;
        assert_eq!(guest.cpu.v[2] >> 32, 0, "SHA1H zeroes the upper lanes");
    }
    let abcd = add4(guest.cpu.v[0], initial);
    let digest = [lane(abcd, 0), lane(abcd, 1), lane(abcd, 2), lane(abcd, 3), e.wrapping_add(initial_e)];
    assert_eq!(digest, [0xa9993e36, 0x4706816a, 0xba3e2571, 0x7850c26c, 0x9cd0d89d]);
}

fn expand_key(key: [u8; 16]) -> Vec<u128> {
    // Host-side key schedule (FIPS-197 5.2); only the cipher rounds run in the guest.
    let mut sbox = [0u8; 256];
    let mul = |mut a: u8, mut b: u8| {
        let mut p = 0u8;
        while b != 0 {
            if b & 1 != 0 {
                p ^= a;
            }
            a = (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 };
            b >>= 1;
        }
        p
    };
    for (x, entry) in sbox.iter_mut().enumerate() {
        let inv = if x == 0 { 0 } else { (1..=255u8).find(|&c| mul(x as u8, c) == 1).unwrap() };
        *entry = inv ^ inv.rotate_left(1) ^ inv.rotate_left(2) ^ inv.rotate_left(3) ^ inv.rotate_left(4) ^ 0x63;
    }
    let mut w: Vec<[u8; 4]> = key.chunks(4).map(|c| c.try_into().unwrap()).collect();
    let mut rcon = 1u8;
    for i in 4..44 {
        let mut t = w[i - 1];
        if i % 4 == 0 {
            t = [sbox[t[1] as usize] ^ rcon, sbox[t[2] as usize], sbox[t[3] as usize], sbox[t[0] as usize]];
            rcon = mul(rcon, 2);
        }
        w.push([0, 1, 2, 3].map(|j| w[i - 4][j] ^ t[j]));
    }
    w.chunks(4)
        .map(|round| u128::from_le_bytes(round.concat().try_into().unwrap()))
        .collect()
}

#[test]
fn aes128_fips197_through_guest_instructions() {
    let key: [u8; 16] = core::array::from_fn(|i| i as u8);
    let plain: [u8; 16] = core::array::from_fn(|i| (i as u8) * 0x11);
    let expected = [
        0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5, 0x5a,
    ];
    let keys = expand_key(key);
    let mut guest = Guest::new();
    guest.cpu.v[0] = u128::from_le_bytes(plain);
    for round in 0..9 {
        guest.cpu.v[1] = keys[round];
        guest.exec(two(0x4e28_4800, 0, 1)); // AESE v0, v1
        guest.exec(two(0x4e28_6800, 0, 0)); // AESMC v0, v0
    }
    guest.cpu.v[1] = keys[9];
    guest.exec(two(0x4e28_4800, 0, 1));
    let cipher = guest.cpu.v[0] ^ keys[10];
    assert_eq!(cipher.to_le_bytes(), expected);

    // Equivalent inverse cipher: AESD + AESIMC with InvMixColumns'd round keys.
    guest.cpu.v[0] = cipher;
    guest.cpu.v[1] = keys[10];
    guest.exec(two(0x4e28_5800, 0, 1)); // AESD v0, v1
    for round in (1..10).rev() {
        guest.exec(two(0x4e28_7800, 0, 0)); // AESIMC v0, v0
        guest.cpu.v[2] = keys[round];
        guest.exec(two(0x4e28_7800, 3, 2)); // AESIMC v3, v2 (key)
        guest.cpu.v[1] = guest.cpu.v[3];
        // IMC(s ^ k) = IMC(s) ^ IMC(k), and AESD xors before inverting.
        guest.exec(two(0x4e28_5800, 0, 1));
    }
    assert_eq!((guest.cpu.v[0] ^ keys[0]).to_le_bytes(), plain);
}

#[test]
fn reserved_crypto_opcodes_are_rejected() {
    let mut guest = Guest::new();
    let page = CODE;
    let word = 0x5e00_7000u32; // SHA three-register opcode 7: unallocated
    guest.memory
        .map(page, [word.to_le_bytes().to_vec(), vec![0; 4092]].concat(), Permissions::READ_EXECUTE)
        .unwrap();
    guest.cpu.pc = page;
    let before = guest.cpu.v;
    assert!(guest.cpu.step(&mut guest.memory).is_err());
    assert_eq!(guest.cpu.v, before);
}

#[test]
fn exclusive_pairs_load_and_store_both_registers() {
    let mut guest = Guest::new();
    guest.memory.map(0x9000, vec![0; 4096], Permissions::READ_WRITE).unwrap();
    guest.memory.write_u64(0x9000, 0x1111_2222_3333_4444).unwrap();
    guest.memory.write_u64(0x9008, 0x5555_6666_7777_8888).unwrap();
    guest.cpu.x[22] = 0x9000;
    guest.exec(0xc87f_a6c8); // LDAXP x8, x9, [x22]
    assert_eq!(guest.cpu.x[8], 0x1111_2222_3333_4444);
    assert_eq!(guest.cpu.x[9], 0x5555_6666_7777_8888);
    guest.cpu.x[1] = 0xaaaa;
    guest.cpu.x[2] = 0xbbbb;
    guest.cpu.x[3] = 7;
    guest.exec(0xc823_8ac1); // STLXP w3, x1, x2, [x22]
    assert_eq!(guest.cpu.x[3], 0);
    assert_eq!(guest.memory.read_u64(0x9000).unwrap(), 0xaaaa);
    assert_eq!(guest.memory.read_u64(0x9008).unwrap(), 0xbbbb);
    guest.exec(0x887f_0ac4); // LDXP w4, w2, [x22]
    assert_eq!(guest.cpu.x[4], 0xaaaa);
    assert_eq!(guest.cpu.x[2], 0);
}
