fn main() {
    let dir = std::env::args().nth(1).expect("output directory");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, stage, code) in emulator_shader::reference::FIXTURES {
        let p = emulator_shader::decode(code, stage).unwrap();
        let words = emulator_shader::emit(&p).unwrap();
        let bytes: Vec<_> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{name}.spv")),
            bytes,
        )
        .unwrap();
        println!(
            "{name}: {} decoded instructions -> {} SPIR-V words",
            p.instruction_count(),
            words.len()
        );
    }
}
