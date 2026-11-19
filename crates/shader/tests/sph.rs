use emulator_shader::sph::{ColorOutput, Header, Kind};

fn header(word0: u32, omap: u32, extra: u32) -> Vec<u8> {
    let mut words = [0u32; 20];
    words[0] = word0;
    words[18] = omap;
    words[19] = extra;
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn z_a_style_pixel_header_fields() {
    let h = Header::parse(&header(0x0006_5462, 0xf, 0)).expect("80 bytes");
    assert_eq!(h.kind(), Kind::Pixel);
    assert_eq!(h.version(), 3);
    assert!(h.mrt_enable());
    assert!(!h.kills_pixels());
    assert_eq!(h.sass_version(), 3);
    let out = h.fragment_outputs().expect("pixel");
    assert_eq!(out.colors.len(), 4);
    assert_eq!(
        out.colors[3],
        ColorOutput {
            target: 0,
            component: 3,
            register: 3
        }
    );
}

#[test]
fn packed_outputs_then_sample_mask_then_depth() {
    // RT0.xy, RT2.w, sample mask, depth.
    let h = Header::parse(&header(2, 0x3 | 0x8 << 8, 0x3)).expect("80 bytes");
    let out = h.fragment_outputs().expect("pixel");
    let regs: Vec<_> = out
        .colors
        .iter()
        .map(|c| (c.target, c.component, c.register))
        .collect();
    assert_eq!(regs, [(0, 0, 0), (0, 1, 1), (2, 3, 2)]);
    assert_eq!(out.sample_mask, Some(3));
    assert_eq!(out.depth, Some(4));
}

#[test]
fn rejects_short_and_non_pixel() {
    assert!(Header::parse(&[0; 79]).is_none());
    let v = Header::parse(&header(1, 0xf, 0)).expect("80 bytes");
    assert_eq!(v.kind(), Kind::Vertex);
    assert!(v.fragment_outputs().is_none());
}
