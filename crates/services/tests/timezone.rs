//! `ITimeZoneService`: own fixed-offset zone data, calendar math and the
//! A/B/C buffer layouts of the rule and conversion commands.
use emulator_a64::Cpu;
use emulator_kernel::{Kernel, SVC_CONNECT_TO_NAMED_PORT, SVC_SEND_SYNC_REQUEST};
use emulator_memory::{AddressSpace, Permissions};
use emulator_services::timezone::{
    decode_rule, encode_rule, find_zone, to_calendar, to_posix, zone_names, TimeZoneService,
    RESULT_TIME_ZONE_CONVERSION_FAILED, RESULT_TIME_ZONE_NOT_FOUND, TIME_ZONE_RULE_SIZE,
};

const TLS: u64 = 0x2000;
const RULE: u64 = 0x10000;
const LIST: u64 = 0x20000;
const POSIX_OUT: u64 = 0x21000;

fn calendar(posix: i64, zone: &str) -> (i16, u8, u8, u8, u8, u8, u32, u32, String, i32) {
    let z = find_zone(zone).unwrap();
    let (offset, abbreviation) = decode_rule(&encode_rule(&z)).unwrap();
    let c = to_calendar(posix, offset, abbreviation).unwrap();
    let name = String::from_utf8_lossy(&c[16..24]).trim_end_matches('\0').to_string();
    (
        i16::from_le_bytes([c[0], c[1]]),
        c[2],
        c[3],
        c[4],
        c[5],
        c[6],
        u32::from_le_bytes(c[8..12].try_into().unwrap()),
        u32::from_le_bytes(c[12..16].try_into().unwrap()),
        name,
        i32::from_le_bytes(c[28..32].try_into().unwrap()),
    )
}

#[test]
fn fixed_zones_and_calendar_math() {
    assert_eq!(calendar(0, "UTC"), (1970, 1, 1, 0, 0, 0, 4, 0, "UTC".into(), 0));
    assert_eq!(calendar(-1, "GMT"), (1969, 12, 31, 23, 59, 59, 3, 364, "GMT".into(), 0));
    // 2000-02-29 (leap day, Tuesday) and 2026-11-17 13:45:30 (Tuesday).
    assert_eq!(calendar(951_782_400, "Etc/UTC"), (2000, 2, 29, 0, 0, 0, 2, 59, "UTC".into(), 0));
    assert_eq!(calendar(1_794_923_130, "UTC"), (2026, 11, 17, 13, 45, 30, 2, 320, "UTC".into(), 0));
    // Etc/GMT+5 is five hours west of UTC.
    assert_eq!(calendar(0, "Etc/GMT+5"), (1969, 12, 31, 19, 0, 0, 3, 364, "-05".into(), -18_000));
    assert_eq!(calendar(0, "Etc/GMT-14").9, 14 * 3600);
    for bad in ["Europe/Berlin", "Etc/GMT+13", "Etc/GMT-15", "Etc/GMT+05", "Etc/GMT+", "utc", ""] {
        assert!(find_zone(bad).is_none(), "{bad}");
    }
    assert_eq!(zone_names().len(), 4 + 12 + 14);
    assert!(zone_names().iter().all(|name| find_zone(name).is_some()));
    // Round trips, including negative times, and mktime-style carry.
    for posix in [-86_401, 0, 951_782_399, 1_794_923_130, 4_102_444_800] {
        let c = to_calendar(posix, -18_000, *b"-05\0\0\0\0\0").unwrap();
        assert_eq!(to_posix(&c, -18_000), Some(posix));
    }
    let month_13 = [0xe9, 0x07, 13, 1, 0, 0, 0, 0]; // 2025-13-01 == 2026-01-01
    assert_eq!(to_posix(&month_13, 0), Some(1_767_225_600));
    assert!(to_calendar(i64::MAX / 2, 0, [0; 8]).is_none(), "year beyond i16");
    assert!(decode_rule(&[0u8; TIME_ZONE_RULE_SIZE]).is_none());
}

struct Rig {
    k: Kernel,
    c: Cpu,
    m: AddressSpace,
    session: u32,
}

impl Rig {
    fn new() -> Self {
        let mut k = Kernel::default();
        let mut c = Cpu::new(0, 0);
        let mut m = AddressSpace::default();
        m.map(0x1000, b"tz\0".to_vec(), Permissions::READ).unwrap();
        m.map(TLS, vec![0; 256], Permissions::READ_WRITE).unwrap();
        m.map(RULE, vec![0xcc; TIME_ZONE_RULE_SIZE], Permissions::READ_WRITE).unwrap();
        m.map(LIST, vec![0; 0x2000], Permissions::READ_WRITE).unwrap();
        k.register_named_port("tz", Box::new(TimeZoneService)).unwrap();
        c.tpidrro_el0 = TLS;
        c.x[1] = 0x1000;
        k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m).unwrap();
        let session = c.x[1] as u32;
        Self { k, c, m, session }
    }

    /// CMIF request: `a` send / `b` receive alias buffers, optional C buffer.
    fn call(&mut self, command: u32, a: &[(u64, u64)], b: &[(u64, u64)], c: Option<(u64, u64)>, data: &[u8]) {
        let mut p = [0u8; 256];
        let word0 = 4 | ((a.len() as u32) << 20) | ((b.len() as u32) << 24);
        let raw_words = 8 + data.len().div_ceil(4) as u32;
        let word1 = raw_words | if c.is_some() { 2 << 10 } else { 0 };
        p[..4].copy_from_slice(&word0.to_le_bytes());
        p[4..8].copy_from_slice(&word1.to_le_bytes());
        let mut at = 8;
        for &(address, size) in a.iter().chain(b) {
            for (i, v) in [size as u32, address as u32, 0].into_iter().enumerate() {
                p[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            at += 12;
        }
        let raw_start = at;
        let header = (at + 15) & !15;
        p[header..header + 4].copy_from_slice(b"SFCI");
        p[header + 8..header + 12].copy_from_slice(&command.to_le_bytes());
        p[header + 16..header + 16 + data.len()].copy_from_slice(data);
        if let Some((address, size)) = c {
            let list = raw_start + raw_words as usize * 4;
            p[list..list + 4].copy_from_slice(&(address as u32).to_le_bytes());
            let fields = ((address >> 32) as u32 & 0xffff) | ((size as u32) << 16);
            p[list + 4..list + 8].copy_from_slice(&fields.to_le_bytes());
        }
        self.m.write(TLS, &p).unwrap();
        self.c.x[0] = u64::from(self.session);
        self.k.dispatch(SVC_SEND_SYNC_REQUEST, &mut self.c, &mut self.m).unwrap();
    }

    fn result(&self) -> u32 {
        self.m.read_u32(TLS + 0x18).unwrap()
    }
    fn out(&self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; length];
        self.m.read(TLS + 0x20, &mut bytes).unwrap();
        bytes
    }
}

fn name(text: &str) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x24];
    bytes[..text.len()].copy_from_slice(text.as_bytes());
    bytes
}

#[test]
fn rules_and_conversions_over_ipc() {
    let mut rig = Rig::new();
    rig.call(0, &[], &[], None, &[]);
    assert_eq!((rig.result(), &rig.out(4)[..]), (0, &b"UTC\0"[..]));
    rig.call(2, &[], &[], None, &[]);
    assert_eq!(u32::from_le_bytes(rig.out(4).try_into().unwrap()), 30);

    // An unknown zone fails and leaves the rule buffer untouched.
    rig.call(4, &[], &[(RULE, TIME_ZONE_RULE_SIZE as u64)], None, &name("Europe/Berlin"));
    assert_eq!(rig.result(), RESULT_TIME_ZONE_NOT_FOUND);
    assert_eq!(rig.m.read_u32(RULE).unwrap(), 0xcccc_cccc);
    // Converting with a buffer that is not a rule fails explicitly.
    rig.call(100, &[(RULE, TIME_ZONE_RULE_SIZE as u64)], &[], None, &0i64.to_le_bytes());
    assert_eq!(rig.result(), RESULT_TIME_ZONE_CONVERSION_FAILED);

    rig.call(4, &[], &[(RULE, TIME_ZONE_RULE_SIZE as u64)], None, &name("Etc/GMT-9"));
    assert_eq!(rig.result(), 0);
    let mut rule = vec![0u8; TIME_ZONE_RULE_SIZE];
    rig.m.read(RULE, &mut rule).unwrap();
    assert_eq!(decode_rule(&rule).unwrap().0, 9 * 3600);
    rig.call(100, &[(RULE, TIME_ZONE_RULE_SIZE as u64)], &[], None, &1_767_225_600i64.to_le_bytes());
    let out = rig.out(0x20);
    assert_eq!(rig.result(), 0);
    assert_eq!((i16::from_le_bytes([out[0], out[1]]), out[2], out[3], out[4]), (2026, 1, 1, 9));
    assert_eq!(&out[16..19], b"+09");

    // ToPosixTime writes one candidate into the C buffer.
    rig.call(201, &[(RULE, TIME_ZONE_RULE_SIZE as u64)], &[], Some((POSIX_OUT, 16)), &out[..8]);
    assert_eq!(rig.result(), 0);
    assert_eq!(rig.out(4), 1u32.to_le_bytes());
    assert_eq!(rig.m.read_u64(POSIX_OUT).unwrap() as i64, 1_767_225_600);
    rig.call(101, &[], &[], None, &0i64.to_le_bytes());
    assert_eq!(&rig.out(4)[..], &[0xb2, 0x07, 1, 1], "1970-01-01 in UTC");

    // The location list pages through the B buffer.
    rig.call(3, &[], &[(LIST, 2 * 0x24)], None, &1u32.to_le_bytes());
    assert_eq!(rig.out(4), 2u32.to_le_bytes());
    let mut names = vec![0u8; 2 * 0x24];
    rig.m.read(LIST, &mut names).unwrap();
    assert_eq!((&names[..4], &names[0x24..0x2b]), (&b"GMT\0"[..], &b"Etc/UTC"[..]));
    rig.call(3, &[], &[(LIST, 0x24)], None, &99u32.to_le_bytes());
    assert_eq!(rig.out(4), 0u32.to_le_bytes());
}
