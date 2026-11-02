fn main() {
    use emulator_shader::{decode, emit, Stage};
    let dir = std::env::args().nth(1).expect("output directory");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, stage, bytes) in [
        (
            "texture.vert",
            Stage::Vertex,
            include_bytes!("../../../examples/shaders/texture.vert.bin").as_slice(),
        ),
        (
            "texture-transform.vert",
            Stage::Vertex,
            include_bytes!("../../../examples/shaders/texture-transform.vert.bin").as_slice(),
        ),
        (
            "texture.frag",
            Stage::Fragment,
            include_bytes!("../../../examples/shaders/texture.frag.bin").as_slice(),
        ),
        (
            "varying.frag",
            Stage::Fragment,
            include_bytes!("../../../examples/shaders/varying.frag.bin").as_slice(),
        ),
    ] {
        let p = decode(bytes, stage).unwrap();
        let bytes: Vec<u8> = emit(&p)
            .unwrap()
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{name}.spv")),
            bytes,
        )
        .unwrap();
    }
}
