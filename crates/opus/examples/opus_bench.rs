//! Decode-loop benchmark: decodes an `opus_demo` bitstream from memory
//! several times and reports the best run (no file output).
//!
//! usage: opus_bench <in.bit> <rate> <channels> [repetitions]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).expect("read bitstream");
    let rate: u32 = args[2].parse().expect("rate");
    let channels: usize = args[3].parse().expect("channels");
    let reps: usize = args.get(4).map_or(5, |r| r.parse().expect("repetitions"));
    let mut pcm = vec![0i16; 5760 * channels];
    let mut best = f64::MAX;
    let mut samples = 0usize;
    for _ in 0..reps {
        let mut dec = emulator_opus::Decoder::new(rate, channels).expect("decoder");
        let start = std::time::Instant::now();
        let mut pos = 0;
        samples = 0;
        while pos + 8 <= data.len() {
            let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 8;
            let packet = &data[pos..pos + len];
            let res = if len == 0 {
                dec.decode(None, &mut pcm, rate as usize / 50)
            } else {
                dec.decode(Some(packet), &mut pcm, 5760)
            };
            samples += res.unwrap_or(0);
            pos += len;
        }
        best = best.min(start.elapsed().as_secs_f64());
    }
    std::hint::black_box(&pcm);
    println!(
        "emulator-opus {best:.4}s ({:.0}x realtime)",
        samples as f64 / f64::from(rate) / best
    );
}
