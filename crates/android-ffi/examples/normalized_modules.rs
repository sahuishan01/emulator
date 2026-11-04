fn main() {
    let directory = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&directory).unwrap();
    for i in 0..11 {
        let replay = emulator_android_ffi::emulator_texture_normalized_replay_v1(i);
        assert_eq!(
            replay.status,
            0,
            "{}",
            String::from_utf8_lossy(&replay.error)
        );
        for n in 0..replay.draw_count as usize {
            let draw = &replay.draws[n].draw;
            let words = &draw.vertex[..draw.vertex_words as usize];
            let raw: Vec<u8> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(directory.join(format!("normalized-{i}-{n}.vert.spv")), raw).unwrap();
        }
    }
}
