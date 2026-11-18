//! Regression test against expectations produced by the reference
//! implementation (see `tools/opus_fixture.py`): exact final ranges for every
//! packet, sample-identical SILK output (the SILK path is fixed point), and
//! CELT/hybrid output within float rounding of the reference.

use emulator_opus::{Decoder, MultiStreamDecoder};

struct Op {
    kind: u8,
    packet: Vec<u8>,
    final_range: u32,
    pcm: Vec<i16>,
}

struct Stream {
    name: String,
    rate: u32,
    channels: usize,
    multistream: Option<(usize, usize, Vec<u8>)>,
    ops: Vec<Op>,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        a
    }
    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
}

fn load() -> Vec<Stream> {
    let data = include_bytes!("data/libopus_fixture.bin");
    let mut r = Reader(data);
    assert_eq!(r.take(4), b"OPFX");
    assert_eq!(r.u32(), 1);
    let n = r.u32();
    (0..n)
        .map(|_| {
            let len = r.u8() as usize;
            let name = String::from_utf8(r.take(len).to_vec()).unwrap();
            let rate = r.u32();
            let channels = r.u32() as usize;
            let streams = r.u32() as usize;
            let coupled = r.u32() as usize;
            let multistream = (streams != 0).then(|| (streams, coupled, r.take(channels).to_vec()));
            let ops = (0..r.u32())
                .map(|_| {
                    let kind = r.u8();
                    let plen = r.u32() as usize;
                    let final_range = r.u32();
                    let packet = r.take(plen).to_vec();
                    let samples = r.u32() as usize;
                    let pcm = r
                        .take(samples * 2)
                        .chunks(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    Op { kind, packet, final_range, pcm }
                })
                .collect();
            Stream { name, rate, channels, multistream, ops }
        })
        .collect()
}

fn compare(name: &str, index: usize, want: &[i16], got: &[i16], exact: bool) {
    assert_eq!(want.len(), got.len(), "{name} op {index}: sample count");
    if exact {
        if let Some(i) = want.iter().zip(got).position(|(a, b)| a != b) {
            let n = want.iter().zip(got).filter(|(a, b)| a != b).count();
            if std::env::var("OPUS_FIXTURE_DEBUG").is_ok() {
                for (k, (a, b)) in want.iter().zip(got).enumerate().filter(|(_, (a, b))| a != b).take(40) {
                    eprintln!("  [{k}] ch{} sample {}: want {a} got {b}", k % 2, k / 2);
                }
            }
            panic!("{name} op {index}: SILK output must match the reference exactly; first diff at {i} ({} vs {}), {n} differ", want[i], got[i]);
        }
        return;
    }
    let mut max = 0;
    let mut sig = 0f64;
    let mut err = 0f64;
    for (&a, &b) in want.iter().zip(got) {
        let d = (i32::from(a) - i32::from(b)).abs();
        max = max.max(d);
        sig += f64::from(a) * f64::from(a);
        err += f64::from(d) * f64::from(d);
    }
    let snr = if err == 0.0 { f64::INFINITY } else { 10.0 * (sig.max(1.0) / err).log10() };
    assert!(max <= 4 && snr >= 50.0, "{name} op {index}: max diff {max}, SNR {snr:.1} dB");
}

#[test]
fn matches_reference_decoder_output() {
    let streams = load();
    assert!(streams.len() >= 13);
    for s in &streams {
        let exact = s.name.starts_with("silk");
        let mut pcm = vec![0i16; 5760 * s.channels];
        if let Some((streams, coupled, mapping)) = &s.multistream {
            let mut dec = MultiStreamDecoder::new(s.rate, s.channels, *streams, *coupled, mapping).unwrap();
            for (i, op) in s.ops.iter().enumerate() {
                assert_eq!(op.kind, 0);
                let n = dec.decode(Some(&op.packet), &mut pcm, 5760).unwrap();
                assert_eq!(dec.final_range(), op.final_range, "{} op {i}: final range", s.name);
                compare(&s.name, i, &op.pcm, &pcm[..n * s.channels], exact);
            }
            continue;
        }
        let mut dec = Decoder::new(s.rate, s.channels).unwrap();
        for (i, op) in s.ops.iter().enumerate() {
            let want_samples = op.pcm.len() / s.channels;
            let n = match op.kind {
                0 => {
                    let n = dec.decode(Some(&op.packet), &mut pcm, 5760).unwrap();
                    assert_eq!(dec.final_range(), op.final_range, "{} op {i}: final range", s.name);
                    n
                }
                1 => dec.decode(None, &mut pcm, want_samples).unwrap(),
                2 => dec.decode_fec(&op.packet, &mut pcm, want_samples).unwrap(),
                k => panic!("unknown op {k}"),
            };
            compare(&s.name, i, &op.pcm, &pcm[..n * s.channels], exact);
        }
    }
}
