use emulator_gpu::{ChannelProcessor, GpuAddressSpace, GpuVa};
use emulator_memory::{AddressSpace, Permissions};
fn rig(index: usize) -> (AddressSpace, GpuAddressSpace, Vec<u8>) {
    let page = [
        include_bytes!("../../../examples/gpu/draw-rgba-page.bin"),
        include_bytes!("../../../examples/gpu/draw-bgra-page.bin"),
        include_bytes!("../../../examples/gpu/draw-scissor-page.bin"),
    ][index];
    let mut cpu = AddressSpace::default();
    cpu.map(0x1000, page.to_vec(), Permissions::READ_WRITE)
        .unwrap();
    let mut gpu = GpuAddressSpace::new(8192).unwrap();
    gpu.map(
        GpuVa(1 << 32),
        cpu.pin(0x1000, 4096, Permissions::READ_WRITE).unwrap(),
    )
    .unwrap();
    let commands = include_bytes!("../../../examples/gpu/draw-rgba-commands.bin");
    let entry = [0u32, 1 | ((commands.len() as u32 / 4) << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    (cpu, gpu, entry)
}
#[test]
fn full_guest_commands_headers_shaders_and_geometry_match_every_pixel() {
    for index in 0..3 {
        let (cpu, gpu, entry) = rig(index);
        let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
        assert_eq!(work.draws().len(), 2);
        assert_eq!(work.color_clears().len(), 1);
        assert_eq!(work.draws()[0].vertex.required_registers(), 4);
        assert_eq!(work.draws()[0].fragment.required_registers(), 9);
        let stats = work.commit().1;
        assert_eq!(stats.draw_launches, 2);
        let mut bytes = [0; 512];
        cpu.read(0x1800, &mut bytes).unwrap();
        for y in 0..8 {
            for x in 0..16 {
                let mut p = if (4..12).contains(&x) && (2..6).contains(&y) && (index != 2 || x < 8)
                {
                    [255, 0, 0, 255]
                } else {
                    [0, 255, 0, 255]
                };
                if index == 1 {
                    p.swap(0, 2);
                }
                assert_eq!(
                    &bytes[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4],
                    &p,
                    "variant{index} pixel{x},{y}"
                );
            }
        }
    }
}
#[test]
fn late_invalid_method_cancels_clear_draw_pixels_and_channel_state() {
    let (mut cpu, gpu, mut entry) = rig(0);
    let count = include_bytes!("../../../examples/gpu/draw-rgba-commands.bin").len() as u32 / 4;
    entry[4..8].copy_from_slice(&(1 | ((count + 2) << 10)).to_le_bytes());
    assert!(ChannelProcessor::default().prepare(&gpu, &entry).is_err());
    let mut bytes = [0; 512];
    cpu.read(0x1800, &mut bytes).unwrap();
    assert_eq!(bytes, [0; 512]);
    // Immutable input snapshots cannot be redirected through CPU unmap/address reuse.
    entry[4..8].copy_from_slice(&(1 | (count << 10)).to_le_bytes());
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
    cpu.unmap(0x1000, 4096).unwrap();
    cpu.map(0x1000, vec![0x55; 4096], Permissions::READ_WRITE)
        .unwrap();
    work.commit();
    cpu.read(0x1800, &mut bytes).unwrap();
    assert_eq!(bytes, [0x55; 512]);
    gpu.read(GpuVa((1 << 32) + 0x800), &mut bytes).unwrap();
    assert_eq!(&bytes[..4], &[0, 255, 0, 255]);
}
#[test]
fn source_mutation_after_prepare_does_not_change_owned_shader_vertex_plan() {
    let (mut cpu, gpu, entry) = rig(0);
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
    let module = work.draws()[0].vertex_spirv.clone();
    cpu.write(0x1a00, &[0; 256]).unwrap();
    cpu.write(0x1d00, &[0; 96]).unwrap();
    assert_eq!(work.draws()[0].vertex_spirv, module);
    work.commit();
    let mut p = [0; 4];
    cpu.read(0x1800, &mut p).unwrap();
    assert_eq!(p, [0, 255, 0, 255]);
}
#[test]
fn shader_vertex_target_feedback_is_rejected_using_backing_identity() {
    let (mut cpu, mut gpu, entry) = rig(0);
    gpu.map(
        GpuVa(2 << 32),
        cpu.pin(0x1000, 4096, Permissions::READ_WRITE).unwrap(),
    )
    .unwrap();
    // Target aliases shader header via a different GPU mapping.
    cpu.write_u32(0x1000 + 2 * 8 + 4, 0xa00).unwrap();
    cpu.write_u32(0x1000 + 8 + 4, 2).unwrap();
    assert!(ChannelProcessor::default()
        .prepare(&gpu, &entry)
        .unwrap_err()
        .contains("DrawInputFeedback"));
}

#[test]
fn dma_after_draw_observes_rendered_bytes_in_submission_order() {
    let (mut cpu, gpu, mut entry) = rig(0);
    let mut commands = include_bytes!("../../../examples/gpu/draw-rgba-commands.bin").to_vec();
    for (offset, data) in [
        (0, 0xb0b5),
        (0x400, 1),
        (0x404, 0x800),
        (0x408, 1),
        (0x40c, 0x8a0),
        (0x418, 4),
        (0x300, 0x182),
    ] {
        commands.extend(
            [0x20010000u32 | (offset / 4), data]
                .into_iter()
                .flat_map(u32::to_le_bytes),
        );
    }
    cpu.write(0x1000, &commands).unwrap();
    entry[4..8].copy_from_slice(&(1 | ((commands.len() as u32 / 4) << 10)).to_le_bytes());
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
    let stats = work.commit().1;
    assert_eq!(
        (
            stats.clear_launches,
            stats.draw_launches,
            stats.copy_launches
        ),
        (1, 2, 1)
    );
    let mut pixel = [0; 4];
    cpu.read(0x18a0, &mut pixel).unwrap();
    assert_eq!(pixel, [0, 255, 0, 255]);
}
