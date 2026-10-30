use emulator_texture::{decode_bc1_block, decode_bc1_rgba8, DecodeError};

fn block(first: u16, second: u16, indices: u32) -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&first.to_le_bytes());
    bytes[2..4].copy_from_slice(&second.to_le_bytes());
    bytes[4..].copy_from_slice(&indices.to_le_bytes());
    bytes
}

#[test]
fn four_color_red_green_vector_and_row_major_selectors() {
    let pixels = decode_bc1_block(&block(0xf800, 0x07e0, 0xe4e4e4e4));
    let row = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [170, 85, 0, 255],
        [85, 170, 0, 255],
    ];
    for y in 0..4 {
        assert_eq!(pixels[y * 4..y * 4 + 4], row);
    }
}

#[test]
fn three_color_blue_red_vector_has_transparent_black() {
    let pixels = decode_bc1_block(&block(0x001f, 0xf800, 0xe4e4e4e4));
    assert_eq!(
        &pixels[..4],
        &[
            [0, 0, 255, 255],
            [255, 0, 0, 255],
            [128, 0, 128, 255],
            [0, 0, 0, 0]
        ]
    );
}

#[test]
fn equal_endpoints_use_three_color_mode_and_keep_opaque_black_distinct() {
    let pixels = decode_bc1_block(&block(0, 0, 0xe4e4e4e4));
    assert_eq!(
        &pixels[..4],
        &[[0, 0, 0, 255], [0, 0, 0, 255], [0, 0, 0, 255], [0, 0, 0, 0]]
    );
}

#[test]
fn low_rgb565_channels_are_normalized_before_interpolation() {
    let pixels = decode_bc1_block(&block(0x0821, 0, 0xe4e4e4e4));
    assert_eq!(
        &pixels[..4],
        &[
            [8, 4, 8, 255],
            [0, 0, 0, 255],
            [5, 3, 5, 255],
            [3, 1, 3, 255]
        ]
    );
}

#[test]
fn image_layout_and_partial_edge_blocks_are_cropped() {
    let mut input = Vec::new();
    for color in [0xf800, 0x07e0, 0x001f, 0xffff] {
        input.extend_from_slice(&block(color, color, 0));
    }
    let pixels = decode_bc1_rgba8(&input, 7, 5).unwrap();
    assert_eq!(pixels.len(), 7 * 5 * 4);
    for y in 0..5 {
        for x in 0..7 {
            let expected = match (x < 4, y < 4) {
                (true, true) => [255, 0, 0, 255],
                (false, true) => [0, 255, 0, 255],
                (true, false) => [0, 0, 255, 255],
                (false, false) => [255, 255, 255, 255],
            };
            assert_eq!(pixels[(y * 7 + x) * 4..(y * 7 + x + 1) * 4], expected);
        }
    }
}

#[test]
fn malformed_and_unbounded_images_fail_before_allocation() {
    assert_eq!(
        decode_bc1_rgba8(&[], 0, 4),
        Err(DecodeError::InvalidDimensions)
    );
    for size in [0, 7, 9, 16] {
        assert!(matches!(
            decode_bc1_rgba8(&vec![0; size], 4, 4),
            Err(DecodeError::InputLength { .. })
        ));
    }
    assert!(matches!(
        decode_bc1_rgba8(&[], u32::MAX, u32::MAX),
        Err(DecodeError::SizeOverflow | DecodeError::AllocationLimit)
    ));
    assert_eq!(
        decode_bc1_rgba8(&[], 16384, 16384),
        Err(DecodeError::AllocationLimit)
    );
}
