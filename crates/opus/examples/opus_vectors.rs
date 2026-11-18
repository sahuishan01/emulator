//! Decode an Opus conformance bitstream (`opus_demo` format: per packet a
//! big-endian u32 length, u32 encoder final range, then the payload) and
//! check every packet's final range. Writes 16-bit little-endian PCM.
//!
//! usage: opus_vectors <in.bit> <rate> <channels> <out.pcm>

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!("usage: opus_vectors <in.bit> <rate> <channels> <out.pcm>");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read bitstream");
    let rate: u32 = args[2].parse().expect("rate");
    let channels: usize = args[3].parse().expect("channels");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[4]).expect("create output"));
    let mut dec = emulator_opus::Decoder::new(rate, channels).expect("decoder");
    let max_frame = rate as usize * 120 / 1000;
    let mut pcm = vec![0i16; max_frame * channels];
    let mut pos = 0;
    let mut count = 0usize;
    let mut mismatches = 0usize;
    let mut lost_prev = true;
    let mut lost_count = 0usize;
    let start = std::time::Instant::now();
    let mut samples_total = 0usize;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let enc_rng = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().unwrap());
        pos += 8;
        if pos + len > data.len() {
            break;
        }
        let packet = &data[pos..pos + len];
        pos += len;
        let lost = len == 0;
        if lost {
            lost_count += 1;
        } else {
            for fr in 0..=lost_count {
                let res = if fr < lost_count {
                    let n = dec.last_packet_duration();
                    dec.decode(None, &mut pcm, n)
                } else {
                    dec.decode(Some(packet), &mut pcm, max_frame)
                };
                match res {
                    Ok(n) => {
                        samples_total += n;
                        for s in &pcm[..n * channels] {
                            out.write_all(&s.to_le_bytes()).unwrap();
                        }
                    }
                    Err(e) => eprintln!("packet {count}: decode error {e:?}"),
                }
            }
            lost_count = 0;
        }
        let rng = dec.final_range();
        if enc_rng != 0 && !lost && !lost_prev && rng != enc_rng {
            if mismatches < 10 {
                eprintln!("packet {count}: range mismatch enc {enc_rng:#010x} dec {rng:#010x} (len {len}, toc {:#04x})", packet.first().copied().unwrap_or(0));
            }
            mismatches += 1;
        }
        lost_prev = lost;
        count += 1;
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "packets {count} range_mismatches {mismatches} samples {samples_total} decode_time {:.3}s ({:.1}x realtime)",
        secs,
        samples_total as f64 / rate as f64 / secs.max(1e-9)
    );
    if mismatches > 0 {
        std::process::exit(1);
    }
}
