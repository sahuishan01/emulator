use crate::quote;
use emulator_shader::{decode, emit, reference};
#[repr(C)]
pub struct ShaderDraw {
    pub abi_version: u32,
    pub status: u32,
    pub vertex_words: u32,
    pub fragment_words: u32,
    pub vertex: [u32; 2048],
    pub fragment: [u32; 2048],
    pub vertices: [u32; 24],
    pub pixels: [u8; 512],
    pub error: [u8; 256],
}
pub fn draw(index: u32) -> ShaderDraw {
    let mut out = ShaderDraw {
        abi_version: 1,
        status: 1,
        vertex_words: 0,
        fragment_words: 0,
        vertex: [0; 2048],
        fragment: [0; 2048],
        vertices: [0; 24],
        pixels: [0; 512],
        error: [0; 256],
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(index, &mut out)))
        .unwrap_or_else(|_| Err("Shader bridge panic".into()));
    if let Err(error) = result {
        out.vertex_words = 0;
        out.fragment_words = 0;
        let n = error.len().min(out.error.len() - 1);
        out.error[..n].copy_from_slice(&error.as_bytes()[..n]);
    }
    out
}
fn build(index: u32, out: &mut ShaderDraw) -> Result<(), String> {
    if index > 2 {
        return Err("Unknown shader fixture".into());
    }
    let vert = if index == 1 { 1 } else { 0 };
    let frag = if index != 0 { 3 } else { 2 };
    for (fixture, words, count) in [
        (vert, &mut out.vertex, &mut out.vertex_words),
        (frag, &mut out.fragment, &mut out.fragment_words),
    ] {
        let (_, stage, bytes) = reference::FIXTURES[fixture];
        let p = decode(bytes, stage).map_err(|e| e.to_string())?;
        let spv = emit(&p).map_err(|e| e.to_string())?;
        words[..spv.len()].copy_from_slice(&spv);
        *count = spv.len() as u32;
    }
    let vertices: [f32; 24] = [
        -1., -1., 0., 1., 1., -1., 0., 1., -1., 1., 0., 1., -1., 1., 0., 1., 1., -1., 0., 1., 1.,
        1., 0., 1.,
    ];
    out.vertices = vertices.map(f32::to_bits);
    for y in 0..8 {
        for x in 0..16 {
            let pixel = if index == 1 && (!(4..12).contains(&x) || !(2..6).contains(&y)) {
                [0, 0, 0, 255]
            } else if index != 0 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            };
            out.pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].copy_from_slice(&pixel);
        }
    }
    out.status = 0;
    Ok(())
}
pub fn report() -> String {
    let cases = reference::run();
    let passed = cases.iter().all(|c| c.passed);
    let tests=cases.iter().map(|c|format!("{{\"name\":{},\"passed\":{},\"decodedInstructions\":{},\"spirvWords\":{},\"verifiedInvocations\":{},\"rejectedPrograms\":{},\"error\":{}}}",quote(c.name),c.passed,c.counts.instructions,c.counts.spirv_words,c.counts.invocations,c.counts.rejected,quote(&c.error))).collect::<Vec<_>>().join(",");
    format!("{{\"scope\":\"original-sm53-shader-reference\",\"suiteVersion\":1,\"hostArchitecture\":{},\"tests\":[{}],\"allPassed\":{}}}",quote(std::env::consts::ARCH),tests,passed)
}
#[cfg(test)]
mod tests {
    #[test]
    fn bridge_layout_bounds_failures_and_independent_pixels() {
        assert_eq!(std::mem::size_of::<super::ShaderDraw>(), 17264);
        assert_eq!(std::mem::align_of::<super::ShaderDraw>(), 4);
        assert_eq!(std::mem::offset_of!(super::ShaderDraw, vertex), 16);
        assert_eq!(std::mem::offset_of!(super::ShaderDraw, fragment), 8208);
        assert_eq!(std::mem::offset_of!(super::ShaderDraw, vertices), 16400);
        assert_eq!(std::mem::offset_of!(super::ShaderDraw, pixels), 16496);
        assert_eq!(std::mem::offset_of!(super::ShaderDraw, error), 17008);
        for index in 0..3 {
            let draw = super::draw(index);
            assert_eq!(draw.status, 0);
            assert!(draw.vertex_words > 0 && draw.vertex_words <= 2048);
            assert!(draw.fragment_words > 0 && draw.fragment_words <= 2048);
            assert_eq!(draw.pixels[3], 255);
            assert!(draw.vertex[draw.vertex_words as usize..]
                .iter()
                .all(|w| *w == 0));
        }
        let failed = super::draw(3);
        assert_ne!(failed.status, 0);
        assert_eq!((failed.vertex_words, failed.fragment_words), (0, 0));
    }
    #[test]
    fn portable_shader_report_contains_actual_bounded_results() {
        let r = crate::emulator_run_shader_checks_v2();
        let s = std::str::from_utf8(&r.bytes[..r.length as usize]).unwrap();
        assert!(s.ends_with("\"allPassed\":true}"), "{s}");
        assert_eq!(s.matches("\"passed\":true").count(), 15);
        assert!(r.bytes[r.length as usize..].iter().all(|b| *b == 0));
    }
}
