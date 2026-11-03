use emulator_texture::{
    decode_bc1_rgba8, decode_bc3_block, decode_bc3_rgba8, unpack_2d, StorageLayout,
};
#[test]
fn bc3_alpha_modes_cross_byte_indices_and_forced_opaque_color() {
    for (a, b, expected) in [
        (255, 0, [255, 0, 219, 182, 146, 109, 73, 36]),
        (20, 200, [20, 200, 56, 92, 128, 164, 0, 255]),
    ] {
        let mut block = [0; 16];
        block[0] = a;
        block[1] = b;
        let indices = (0..16).fold(0u64, |bits, i| bits | ((i % 8) as u64) << (i * 3));
        block[2..8].copy_from_slice(&indices.to_le_bytes()[..6]);
        // c0<c1 and index3: BC1 would be transparent; BC3 must use four colors.
        block[8..12].copy_from_slice(&[0xe0, 7, 0, 0xf8]);
        block[12..].fill(255);
        for (i, p) in decode_bc3_block(&block).iter().enumerate() {
            assert_eq!(*p, [170, 85, 0, expected[i % 8]]);
        }
    }
    assert!(decode_bc3_rgba8(&[], 0, 1).is_err());
    assert!(decode_bc3_rgba8(&[0; 15], 4, 4).is_err());
    assert!(decode_bc3_rgba8(&[], u32::MAX, u32::MAX).is_err());
}
#[test]
fn independent_owned_storage_pages_match_all_canonical_bytes() {
    type Profile = (
        &'static [u8],
        &'static [u8],
        usize,
        usize,
        u32,
        u32,
        u32,
        Option<u8>,
    );
    let profiles: [Profile; 6] = [
        (
            include_bytes!("../../../examples/gpu/storage-tiled-rgba-source.bin"),
            include_bytes!("../../../examples/gpu/storage-tiled-rgba-rgba"),
            16,
            4,
            4,
            4,
            8,
            Some(0),
        ),
        (
            include_bytes!("../../../examples/gpu/storage-multigob-rgba-source.bin"),
            include_bytes!("../../../examples/gpu/storage-multigob-rgba-rgba"),
            68,
            33,
            17,
            33,
            8,
            Some(1),
        ),
        (
            include_bytes!("../../../examples/gpu/storage-pitch-bc1-source.bin"),
            include_bytes!("../../../examples/gpu/storage-pitch-bc1-rgba"),
            16,
            2,
            7,
            5,
            0x24,
            None,
        ),
        (
            include_bytes!("../../../examples/gpu/storage-tiled-bc1-alpha-source.bin"),
            include_bytes!("../../../examples/gpu/storage-tiled-bc1-alpha-rgba"),
            16,
            3,
            7,
            9,
            0x24,
            Some(0),
        ),
        (
            include_bytes!("../../../examples/gpu/storage-pitch-bc3-source.bin"),
            include_bytes!("../../../examples/gpu/storage-pitch-bc3-rgba"),
            32,
            2,
            5,
            7,
            0x26,
            None,
        ),
        (
            include_bytes!("../../../examples/gpu/storage-tiled-bc3-alpha-source.bin"),
            include_bytes!("../../../examples/gpu/storage-tiled-bc3-alpha-rgba"),
            64,
            3,
            13,
            9,
            0x26,
            Some(2),
        ),
    ];
    for (input, expected, row, rows, w, h, fmt, log) in profiles {
        let layout = log.map_or(
            StorageLayout::Pitch {
                pitch: row.div_ceil(32) * 32,
            },
            |height_log2| StorageLayout::BlockLinear { height_log2 },
        );
        let packed = unpack_2d(input, row, rows, layout).unwrap();
        let actual = match fmt {
            8 => packed,
            0x24 => decode_bc1_rgba8(&packed, w, h).unwrap(),
            _ => decode_bc3_rgba8(&packed, w, h).unwrap(),
        };
        assert_eq!(actual, expected, "{w}x{h} fmt{fmt:x}");
    }
}
