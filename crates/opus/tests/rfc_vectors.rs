//! RFC 6716 / RFC 8251 conformance vectors. The vectors are not in the
//! repository; set `OPUS_TESTVECTORS` to the extracted
//! `opus_testvectors-rfc8251` directory to run this test.

use emulator_opus::Decoder;

fn decode(bits: &[u8], rate: u32, channels: usize) -> (Vec<i16>, usize) {
    let mut dec = Decoder::new(rate, channels).unwrap();
    let max = rate as usize * 120 / 1000;
    let mut pcm = vec![0i16; max * channels];
    let mut out = Vec::new();
    let mut pos = 0;
    let mut mismatches = 0;
    let mut lost_prev = true;
    while pos + 8 <= bits.len() {
        let len = u32::from_be_bytes(bits[pos..pos + 4].try_into().unwrap()) as usize;
        let rng = u32::from_be_bytes(bits[pos + 4..pos + 8].try_into().unwrap());
        pos += 8;
        let packet = &bits[pos..pos + len];
        pos += len;
        let lost = len == 0;
        let n = if lost {
            dec.decode(None, &mut pcm, dec.last_packet_duration()).unwrap()
        } else {
            dec.decode(Some(packet), &mut pcm, max).unwrap()
        };
        out.extend_from_slice(&pcm[..n * channels]);
        if !lost && !lost_prev && rng != 0 && dec.final_range() != rng {
            mismatches += 1;
        }
        lost_prev = lost;
    }
    (out, mismatches)
}

#[test]
fn rfc8251_vectors() {
    let Ok(dir) = std::env::var("OPUS_TESTVECTORS") else {
        eprintln!("OPUS_TESTVECTORS not set; skipping the RFC 8251 vectors");
        return;
    };
    for i in 1..=12 {
        let bits = std::fs::read(format!("{dir}/testvector{i:02}.bit")).unwrap();
        let reference = std::fs::read(format!("{dir}/testvector{i:02}.dec")).unwrap();
        let reference: Vec<i16> = reference.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        let (out, mismatches) = decode(&bits, 48_000, 2);
        assert_eq!(mismatches, 0, "vector {i}: final range mismatches");
        assert_eq!(out.len(), reference.len(), "vector {i}: length");
        let (mut sig, mut err) = (0f64, 0f64);
        for (&a, &b) in reference.iter().zip(&out) {
            sig += f64::from(a).powi(2);
            err += (f64::from(a) - f64::from(b)).powi(2);
        }
        let snr = 10.0 * (sig / err.max(1.0)).log10();
        eprintln!("vector {i}: SNR vs reference {snr:.1} dB");
        assert!(snr > 40.0, "vector {i}: SNR {snr:.1} dB");
    }
}
