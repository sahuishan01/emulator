use emulator_gpu::*;
use emulator_memory::{Access, AddressSpace, Permissions};
const BASE: u64 = 1 << 32;
const PAGE: &[u8] = include_bytes!("../../../examples/gpu/depth-1-page.bin");
fn setup(page: Vec<u8>) -> (AddressSpace, GpuAddressSpace) {
    let mut cpu = AddressSpace::default();
    cpu.map(0x1000, page, Permissions::READ_WRITE).unwrap();
    let mut gpu = GpuAddressSpace::new(32768).unwrap();
    gpu.map(
        GpuVa(BASE),
        cpu.pin(0x1000, 8192, Permissions::READ_WRITE).unwrap(),
    )
    .unwrap();
    (cpu, gpu)
}
fn entry(words: u32) -> Vec<u8> {
    [0, 1 | (words << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}
fn set(p: &mut [u8], method: u32, value: u32) {
    let i = (0..648)
        .step_by(8)
        .rfind(|&i| u32::from_le_bytes(p[i..i + 4].try_into().unwrap()) & 8191 == method / 4)
        .unwrap();
    p[i + 4..i + 8].copy_from_slice(&value.to_le_bytes());
}
fn pixels(cpu: &AddressSpace) -> [u8; 512] {
    let mut b = [0; 512];
    cpu.read(0x1800, &mut b).unwrap();
    b
}
fn split_depth(cpu: &AddressSpace, gpu: &mut GpuAddressSpace, permission: Permissions) {
    let id = gpu
        .pin(GpuVa(BASE), 1, Access::Read)
        .unwrap()
        .mapping_ids()
        .next()
        .unwrap();
    gpu.unmap(id).unwrap();
    for (address, cpu_address, length, rights) in [
        (BASE, 0x1000, 4096, Permissions::READ_WRITE),
        (BASE + 0x1000, 0x2000, 512, permission),
        (BASE + 0x1200, 0x2200, 3584, Permissions::READ_WRITE),
    ] {
        gpu.map(
            GpuVa(address),
            cpu.pin(cpu_address, length, rights).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn later_depth_draw_reads_earlier_depth_effect_without_early_publication() {
    let (cpu, gpu) = setup(PAGE.to_vec());
    let (_, work) = ChannelProcessor::default()
        .prepare(&gpu, &entry(162))
        .unwrap();
    assert_eq!(
        work.draws()[0].depth.as_ref().unwrap().initial,
        vec![1f32.to_bits(); 128]
    );
    assert_eq!(
        work.draws()[1].depth.as_ref().unwrap().initial,
        vec![0.25f32.to_bits(); 128]
    );
    assert_eq!(pixels(&cpu), [0; 512]);
    work.commit();
    assert!(pixels(&cpu).chunks_exact(4).all(|p| p == [0, 255, 0, 255]));
}
#[test]
fn prepared_depth_and_vertices_are_owned_across_cpu_changes() {
    let (mut cpu, gpu) = setup(PAGE.to_vec());
    let (_, work) = ChannelProcessor::default()
        .prepare(&gpu, &entry(162))
        .unwrap();
    cpu.write(0x2000, &[0; 512]).unwrap();
    cpu.write(0x1d00, &[0; 96]).unwrap();
    work.commit();
    assert!(pixels(&cpu).chunks_exact(4).all(|p| p == [0, 255, 0, 255]));
    let mut raw = [0; 512];
    cpu.read(0x2000, &mut raw).unwrap();
    assert!(raw.chunks_exact(4).all(|v| v == 0.25f32.to_le_bytes()));
}
#[test]
fn read_only_depth_succeeds_without_write_permission_and_write_enabled_fails() {
    let page = include_bytes!("../../../examples/gpu/depth-8-page.bin");
    let (cpu, mut gpu) = setup(page.to_vec());
    split_depth(&cpu, &mut gpu, Permissions::READ);
    let (_, work) = ChannelProcessor::default()
        .prepare(&gpu, &entry(162))
        .unwrap();
    work.commit();
    let mut raw = [0; 512];
    cpu.read(0x2000, &mut raw).unwrap();
    assert_eq!(raw.as_slice(), &page[0x1000..0x1200]);
    let (cpu, mut gpu) = setup(PAGE.to_vec());
    split_depth(&cpu, &mut gpu, Permissions::READ);
    assert!(ChannelProcessor::default()
        .prepare(&gpu, &entry(162))
        .is_err());
    assert_eq!(pixels(&cpu), [0; 512]);
}
#[test]
fn write_only_depth_is_rejected_before_any_color_write() {
    let (cpu, mut gpu) = setup(PAGE.to_vec());
    split_depth(&cpu, &mut gpu, Permissions::WRITE);
    assert!(ChannelProcessor::default()
        .prepare(&gpu, &entry(162))
        .is_err());
    assert_eq!(pixels(&cpu), [0; 512]);
}
#[test]
fn unsupported_varying_depth_and_invalid_second_draw_cancel_the_entire_batch() {
    for z in [0.5f32, f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let mut page = PAGE.to_vec();
        page[0x1308..0x130c].copy_from_slice(&z.to_le_bytes());
        let (cpu, gpu) = setup(page);
        assert!(ChannelProcessor::default()
            .prepare(&gpu, &entry(162))
            .is_err());
        assert_eq!(pixels(&cpu), [0; 512]);
        let mut raw = [0; 512];
        cpu.read(0x2000, &mut raw).unwrap();
        assert_eq!(raw.as_slice(), &PAGE[0x1000..0x1200]);
    }
}
#[test]
fn depth_budget_boundary_and_drop_publish_nothing() {
    let (cpu, gpu) = setup(PAGE.to_vec());
    let prepare = |budget| {
        let mut q = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
        let id = q.submit(&gpu, &entry(162)).unwrap();
        q.prepare_front(
            &gpu,
            id,
            ExecutionLimits {
                bytes: budget,
                launches: 64,
            },
        )
    };
    let (mut low, mut high) = (1, 4 * 1024 * 1024);
    while low < high {
        let mid = (low + high) / 2;
        if prepare(mid).is_ok() {
            high = mid
        } else {
            low = mid + 1
        }
    }
    assert!(matches!(prepare(low - 1), Err(ExecutionError::Capacity)));
    let work = prepare(low).unwrap();
    drop(work);
    assert_eq!(pixels(&cpu), [0; 512]);
    prepare(low).unwrap().commit();
    assert!(pixels(&cpu).chunks_exact(4).all(|p| p == [0, 255, 0, 255]));
}
#[test]
fn fragmented_attachment_span_limit_rolls_back_late_depth_draws() {
    let mut page = PAGE.to_vec();
    for at in [648, 656] {
        page[at..at + 8].copy_from_slice(&[0x85, 0x04, 0x01, 0x20, 0, 0, 6, 0x40]);
    }
    let (cpu, mut gpu) = setup(page);
    let id = gpu
        .pin(GpuVa(BASE), 1, Access::Read)
        .unwrap()
        .mapping_ids()
        .next()
        .unwrap();
    gpu.unmap(id).unwrap();
    gpu.map(
        GpuVa(BASE),
        cpu.pin(0x1000, 4096, Permissions::READ_WRITE).unwrap(),
    )
    .unwrap();
    for i in 0..512 {
        gpu.map(
            GpuVa(BASE + 0x1000 + i),
            cpu.pin(0x2000 + i, 1, Permissions::READ_WRITE).unwrap(),
        )
        .unwrap();
    }
    gpu.map(
        GpuVa(BASE + 0x1200),
        cpu.pin(0x2200, 3584, Permissions::READ_WRITE).unwrap(),
    )
    .unwrap();
    assert!(ChannelProcessor::default()
        .prepare(&gpu, &entry(166))
        .unwrap_err()
        .contains("Capacity"));
    assert_eq!(pixels(&cpu), [0; 512]);
}
#[test]
fn depth_backend_is_explicit_and_color_alias_feedback_is_rejected() {
    let (cpu, gpu) = setup(PAGE.to_vec());
    let (_, work) = ChannelProcessor::default()
        .prepare(&gpu, &entry(162))
        .unwrap();
    assert!(work.graphics_frame().unwrap_err().contains("depth replay"));
    drop(work);
    assert_eq!(pixels(&cpu), [0; 512]);
    let mut p = PAGE.to_vec();
    set(&mut p, 0xfe4, 0x800);
    set(&mut p, 0xd8c, 0);
    let (cpu, gpu) = setup(p);
    assert!(ChannelProcessor::default()
        .prepare(&gpu, &entry(162))
        .unwrap_err()
        .contains("DrawInputFeedback"));
    assert_eq!(pixels(&cpu), [0; 512]);
}
