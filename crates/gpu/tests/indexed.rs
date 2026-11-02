use emulator_gpu::{
    ChannelProcessor, ExecutionLimits, GpuAddressSpace, GpuVa, QueueLimits, SubmissionQueue,
};
use emulator_memory::{AddressSpace, Permissions};
const PAGE: &[u8] = include_bytes!("../../../examples/gpu/indexed-u8-page.bin");
const WORDS: u32 = include_bytes!("../../../examples/gpu/indexed-u8-commands.bin").len() as u32 / 4;
fn setup() -> (AddressSpace, GpuAddressSpace, Vec<u8>) {
    let mut cpu = AddressSpace::default();
    cpu.map(0x1000, PAGE.to_vec(), Permissions::READ_WRITE)
        .unwrap();
    let mut gpu = GpuAddressSpace::new(8192).unwrap();
    gpu.map(
        GpuVa(1 << 32),
        cpu.pin(0x1000, 4096, Permissions::READ_WRITE).unwrap(),
    )
    .unwrap();
    (
        cpu,
        gpu,
        [0u32, 1 | (WORDS << 10)]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect(),
    )
}
#[test]
fn indexed_effect_and_capture_budgets_cancel_without_pixels() {
    let (cpu, gpu, entry) = setup();
    for limits in [
        ExecutionLimits {
            bytes: 512,
            launches: 64,
        },
        ExecutionLimits {
            bytes: 4 * 1024 * 1024,
            launches: 1,
        },
        ExecutionLimits {
            bytes: 4 * 1024 * 1024,
            launches: 2,
        },
    ] {
        let mut q = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
        let id = q.submit(&gpu, &entry).unwrap();
        assert!(q
            .prepare_front(&gpu, id, limits)
            .unwrap_err()
            .to_string()
            .contains("Capacity"));
        let mut pixels = [0; 512];
        cpu.read(0x1800, &mut pixels).unwrap();
        assert_eq!(pixels, [0; 512]);
    }
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
    assert_eq!(
        work.graphics_frame().unwrap().draws[0]
            .indices
            .as_ref()
            .unwrap()
            .values,
        [0, 1, 2, 2, 1, 3]
    );
    drop(work);
    let mut pixels = [0; 512];
    cpu.read(0x1800, &mut pixels).unwrap();
    assert_eq!(pixels, [0; 512]);
}
#[test]
fn indexed_winding_and_unused_vertex_inputs_follow_element_order() {
    let (mut cpu, gpu, entry) = setup();
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
    let original = work.graphics_frame().unwrap().pixels;
    drop(work);
    // Reverse each triangle, including the shared edge. Output must remain identical.
    cpu.write(0x1f00, &[0, 2, 1, 2, 3, 1]).unwrap();
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
    assert_eq!(work.graphics_frame().unwrap().pixels, original);
    drop(work);
    // Unreferenced input2 contains NaN; the three referenced corners remain valid.
    cpu.write(0x1f00, &[0, 1, 3, 0, 1, 3]).unwrap();
    cpu.write_u32(0x1d40, f32::NAN.to_bits()).unwrap();
    let (_, work) = ChannelProcessor::default().prepare(&gpu, &entry).unwrap();
    assert_eq!(work.draws()[0].vertices.len(), 4);
    assert!(work.graphics_frame().is_ok());
}
#[test]
fn unsupported_index_setup_is_explicit_and_does_not_publish_state() {
    let (mut cpu, gpu, entry) = setup();
    // State-driven size/count registers are deliberately outside this packed-command slice.
    cpu.write_u32(0x1000 + WORDS as u64 * 4, 0x20010000 | (0x17d8 / 4))
        .unwrap();
    cpu.write_u32(0x1004 + WORDS as u64 * 4, 3).unwrap();
    let bad = [0u32, 1 | ((WORDS + 2) << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let channel = ChannelProcessor::default();
    assert!(channel
        .prepare(&gpu, &bad)
        .unwrap_err()
        .contains("UnsupportedMethod"));
    let (_, work) = channel.prepare(&gpu, &entry).unwrap();
    assert_eq!(work.draws()[0].indices.as_ref().unwrap().element_bytes, 1);
}
