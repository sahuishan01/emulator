//! `audout:u` over real CMIF requests with A/B name buffers.
use emulator_a64::Cpu;
use emulator_kernel::{service::ServiceError, Kernel, KernelError, SVC_CONNECT_TO_NAMED_PORT, SVC_SEND_SYNC_REQUEST};
use emulator_memory::{AddressSpace, Permissions};
use emulator_services::aud::AudioOutManager;

const TLS: u64 = 0x2000;
const NAME_IN: u64 = 0x3000;
const NAME_OUT: u64 = 0x3200;

struct Rig {
    k: Kernel,
    c: Cpu,
    m: AddressSpace,
    session: u32,
}

fn descriptor(b: &mut [u8], at: usize, address: u64, size: u64) {
    for (i, v) in [size as u32, address as u32, 0].into_iter().enumerate() {
        b[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

impl Rig {
    fn new() -> Self {
        let mut k = Kernel::default();
        let mut c = Cpu::new(0, 0);
        let mut m = AddressSpace::default();
        m.map(0x1000, b"audout:u\0".to_vec(), Permissions::READ).unwrap();
        m.map(TLS, vec![0; 256], Permissions::READ_WRITE).unwrap();
        m.map(NAME_IN, vec![0; 0x1000], Permissions::READ_WRITE).unwrap();
        k.register_named_port("audout:u", Box::new(AudioOutManager)).unwrap();
        c.tpidrro_el0 = TLS;
        c.x[1] = 0x1000;
        k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m).unwrap();
        let session = c.x[1] as u32;
        Self { k, c, m, session }
    }

    /// CMIF request with `a` send and `b` receive alias buffers.
    fn call(&mut self, command: u32, a: &[(u64, u64)], b: &[(u64, u64)], words: &[u32]) -> Result<(), KernelError> {
        let session = self.session;
        self.call_on(session, command, a, b, words)
    }

    fn call_on(&mut self, session: u32, command: u32, a: &[(u64, u64)], b: &[(u64, u64)], words: &[u32]) -> Result<(), KernelError> {
        let mut p = [0u8; 256];
        let word0 = 4 | ((a.len() as u32) << 20) | ((b.len() as u32) << 24);
        p[..4].copy_from_slice(&word0.to_le_bytes());
        p[4..8].copy_from_slice(&(8 + words.len() as u32).to_le_bytes());
        let mut at = 8;
        for &(addr, size) in a.iter().chain(b) {
            descriptor(&mut p, at, addr, size);
            at += 12;
        }
        at = (at + 15) & !15;
        p[at..at + 4].copy_from_slice(b"SFCI");
        p[at + 8..at + 12].copy_from_slice(&command.to_le_bytes());
        for (i, w) in words.iter().enumerate() {
            let o = at + 16 + i * 4;
            p[o..o + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.m.write(TLS, &p).unwrap();
        self.c.x[0] = u64::from(session);
        self.k.dispatch(SVC_SEND_SYNC_REQUEST, &mut self.c, &mut self.m).map(|_| ())
    }

    fn result(&self) -> u32 {
        self.m.read_u32(TLS + 0x18).unwrap()
    }
    fn out_word(&self, i: u64) -> u32 {
        self.m.read_u32(TLS + 0x20 + i * 4).unwrap()
    }
    fn name_out(&self) -> Vec<u8> {
        let mut n = vec![0u8; 9];
        self.m.read(NAME_OUT, &mut n).unwrap();
        n
    }
}

/// sample_rate, channel_count|reserved, ARUID (2 words).
fn params(rate: u32, channels: u32) -> [u32; 4] {
    [rate, channels, 0, 0]
}

#[test]
fn open_audio_out_with_empty_name_selects_default_48k_stereo_stopped() {
    let mut r = Rig::new();
    let names = r.k.live_handle_count();
    r.call(1, &[(NAME_IN, 0x100)], &[(NAME_OUT, 0x100)], &params(0, 0)).unwrap();
    assert_eq!(r.result(), 0);
    // Output words follow the reply header; a moved IAudioOut handle precedes them.
    assert_eq!(r.k.live_handle_count(), names + 1, "IAudioOut session handed out");
    assert_eq!(r.name_out(), b"DeviceOut");
}

#[test]
fn open_audio_out_accepts_named_default_and_rejects_unknown_rate_name_and_channels() {
    let mut r = Rig::new();
    r.m.write(NAME_IN, b"DeviceOut\0").unwrap();
    r.call(1, &[(NAME_IN, 0x100)], &[(NAME_OUT, 0x100)], &params(48_000, 2)).unwrap();
    assert_eq!(r.result(), 0);

    let before = r.k.live_handle_count();
    r.call(1, &[(NAME_IN, 0x100)], &[(NAME_OUT, 0x100)], &params(44_100, 2)).unwrap();
    assert_ne!(r.result(), 0, "only 48 kHz exists");
    r.call(1, &[(NAME_IN, 0x100)], &[(NAME_OUT, 0x100)], &params(48_000, 6)).unwrap();
    assert_ne!(r.result(), 0, "6-channel output is not implemented");
    r.m.write(NAME_IN, b"Headphone\0").unwrap();
    r.call(1, &[(NAME_IN, 0x100)], &[(NAME_OUT, 0x100)], &params(48_000, 2)).unwrap();
    assert_ne!(r.result(), 0, "unknown output name");
    assert_eq!(r.k.live_handle_count(), before, "rejections hand out nothing");
}

#[test]
fn open_audio_out_requires_its_name_buffers_and_payload() {
    let mut r = Rig::new();
    assert!(matches!(
        r.call(1, &[], &[], &params(0, 0)),
        Err(KernelError::Service(ServiceError::InvalidBufferCounts { .. }))
    ));
    assert!(matches!(
        r.call(1, &[(NAME_IN, 0x100)], &[(NAME_OUT, 0x100)], &[0]),
        Err(KernelError::Service(ServiceError::InvalidPayload { .. }))
    ));
}

#[test]
fn list_audio_outs_writes_one_name_record() {
    let mut r = Rig::new();
    r.call(0, &[], &[(NAME_OUT, 0x100)], &[]).unwrap();
    assert_eq!(r.result(), 0);
    assert_eq!(r.out_word(0), 1);
    assert_eq!(r.name_out(), b"DeviceOut");
    // A buffer too small for a record lists nothing.
    r.call(0, &[], &[(NAME_OUT + 0x200, 0x10)], &[]).unwrap();
    assert_eq!(r.out_word(0), 0);
    assert!(r.call(9, &[], &[], &[]).is_err());
}

const RECORD: u64 = 0x3400;
const TAGS: u64 = 0x3600;

impl Rig {
    /// Opens the default out and returns its IAudioOut session handle.
    fn open(&mut self) -> u32 {
        self.call(1, &[(NAME_IN, 0x100)], &[(NAME_OUT, 0x100)], &params(0, 0)).unwrap();
        assert_eq!(self.result(), 0);
        self.m.read_u32(TLS + 12).unwrap()
    }
    /// Reply payload words follow the moved-handle descriptor.
    fn word(&self, i: u64) -> u32 {
        self.m.read_u32(TLS + 0x18 + i * 4).unwrap()
    }
    fn append(&mut self, out: u32, tag: u64, data_size: u64) -> u32 {
        self.m.write_u64(RECORD + 0x18, data_size).unwrap();
        self.call_on(out, 3, &[(RECORD, 0x28)], &[], &[tag as u32, (tag >> 32) as u32]).unwrap();
        self.word(0)
    }
    fn released(&mut self, out: u32) -> Vec<u64> {
        self.call_on(out, 5, &[], &[(TAGS, 0x40)], &[]).unwrap();
        assert_eq!(self.word(0), 0);
        let n = self.word(2) as u64;
        (0..n).map(|i| self.m.read_u64(TAGS + i * 8).unwrap()).collect()
    }
}

#[test]
fn audio_out_null_sink_releases_buffers_once_started() {
    let mut r = Rig::new();
    let out = r.open();
    r.call_on(out, 0, &[], &[], &[]).unwrap();
    assert_eq!(r.word(2), 1, "a fresh out is stopped");

    // Queued while stopped: held, not released.
    assert_eq!(r.append(out, 0xa1, 0x1000), 0);
    assert!(r.released(out).is_empty());
    r.call_on(out, 6, &[], &[], &[0xa1, 0]).unwrap();
    assert_eq!(r.word(2) & 0xff, 1, "ContainsAudioOutBuffer");

    r.call_on(out, 1, &[], &[], &[]).unwrap(); // Start
    assert_eq!(r.append(out, 0xa2, 0x800), 0);
    assert_eq!(r.released(out), vec![0xa1, 0xa2]);
    assert!(r.released(out).is_empty(), "each tag is returned once");
    r.call_on(out, 10, &[], &[], &[]).unwrap();
    assert_eq!(r.word(2), (0x1800 / 4) as u32, "played sample count");
    r.call_on(out, 6, &[], &[], &[0xa1, 0]).unwrap();
    assert_eq!(r.word(2) & 0xff, 0);
}

#[test]
fn audio_out_rejects_short_records_overflow_and_bad_volume() {
    let mut r = Rig::new();
    let out = r.open();
    r.call_on(out, 3, &[(RECORD, 0x10)], &[], &[1, 0]).unwrap();
    assert_ne!(r.word(0), 0, "record shorter than AudioOutBuffer");
    for tag in 0..32 {
        assert_eq!(r.append(out, tag, 0x100), 0);
    }
    assert_ne!(r.append(out, 99, 0x100), 0, "33rd buffer exceeds the cap");
    // Flush returns queued buffers unplayed.
    r.call_on(out, 11, &[], &[], &[]).unwrap();
    assert_eq!(r.word(2) & 0xff, 1);
    r.call_on(out, 10, &[], &[], &[]).unwrap();
    assert_eq!(r.word(2), 0);

    r.call_on(out, 12, &[], &[], &[0.5f32.to_bits()]).unwrap();
    assert_eq!(r.word(0), 0);
    r.call_on(out, 12, &[], &[], &[2.0f32.to_bits()]).unwrap();
    assert_ne!(r.word(0), 0);
    r.call_on(out, 13, &[], &[], &[]).unwrap();
    assert_eq!(f32::from_bits(r.word(2)), 0.5);
    assert!(r.call_on(out, 3, &[], &[], &[1, 0]).is_err(), "missing record buffer");
    assert!(r.call_on(out, 99, &[], &[], &[]).is_err());
}
