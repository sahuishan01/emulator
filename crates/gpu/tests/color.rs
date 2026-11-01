use emulator_gpu::*;
use emulator_memory::{AddressSpace, Permissions};
struct Rig {
    cpu: AddressSpace,
    gpu: GpuAddressSpace,
}
impl Rig {
    fn new() -> Self {
        let mut cpu = AddressSpace::with_limit(8192);
        cpu.map(0x1000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        cpu.map(0x3000, vec![0x55; 64], Permissions::READ_WRITE)
            .unwrap();
        let mut gpu = GpuAddressSpace::new(8192).unwrap();
        gpu.map(
            GpuVa(1 << 32),
            cpu.pin(0x1000, 4096, Permissions::READ).unwrap(),
        )
        .unwrap();
        gpu.map(
            GpuVa(2 << 32),
            cpu.pin(0x3000, 64, Permissions::READ_WRITE).unwrap(),
        )
        .unwrap();
        Self { cpu, gpu }
    }
    fn prepare(&mut self, methods: &[(u32, u32)]) -> Result<PreparedSubmission, ExecutionError> {
        let bytes: Vec<_> = methods
            .iter()
            .flat_map(|(m, v)| [0x20010000 | (m / 4), *v])
            .flat_map(u32::to_le_bytes)
            .collect();
        self.cpu.write(0x1000, &bytes).unwrap();
        let entries: Vec<_> = [0_u32, 1 | ((bytes.len() as u32 / 4) << 10)]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let mut q = SubmissionQueue::new(&self.gpu, QueueLimits::default()).unwrap();
        let id = q.submit(&self.gpu, &entries).unwrap();
        q.prepare_front(&self.gpu, id, ExecutionLimits::default())
    }
    fn pixels(&self) -> [u8; 64] {
        let mut v = [0; 64];
        self.cpu.read(0x3000, &mut v).unwrap();
        v
    }
}
fn clear() -> Vec<(u32, u32)> {
    vec![
        (0, MAXWELL_B),
        (0x800, 2),
        (0x804, 0),
        (0x808, 16),
        (0x80c, 4),
        (0x810, RGBA8_UNORM),
        (0x814, 0x1000),
        (0x818, 1),
        (0x81c, 0),
        (0x820, 0),
        (0xff4, 4 << 16),
        (0xff8, 4 << 16),
        (0x10f8, 0),
        (0xd80, 1_f32.to_bits()),
        (0xd84, 0),
        (0xd88, 0),
        (0xd8c, 1_f32.to_bits()),
        (0x19d0, 0x3c),
    ]
}
#[test]
fn clear_preparation_is_cancelable_and_pins_original_backing_after_cpu_reuse() {
    let mut r = Rig::new();
    let work = r.prepare(&clear()).unwrap();
    assert_eq!(work.color_clears().len(), 1);
    assert_eq!(work.stats().cleared_bytes, 64);
    assert_eq!(r.pixels(), [0x55; 64]);
    drop(work);
    assert_eq!(r.pixels(), [0x55; 64]);
    let work = r.prepare(&clear()).unwrap();
    r.cpu.unmap(0x3000, 64).unwrap();
    r.cpu
        .map(0x3000, vec![9; 64], Permissions::READ_WRITE)
        .unwrap();
    work.commit();
    assert_eq!(r.pixels(), [9; 64]);
    let mut old = [0; 64];
    r.gpu.read(GpuVa(2 << 32), &mut old).unwrap();
    for pixel in old.chunks_exact(4) {
        assert_eq!(pixel, [255, 0, 0, 255]);
    }
}
#[test]
fn mixed_clear_dma_and_masked_write_execute_in_original_method_order() {
    let mut r = Rig::new();
    let mut m = clear();
    m.extend([
        (0x10f8, 0x10),
        (0xd6c, 1 << 16),
        (0xd70, 1 << 16),
        (0xd84, 1_f32.to_bits()),
        (0x19d0, 8),
    ]);
    // Switch class, copy the newly yellow pixel to the final pixel.
    m.extend([
        (0, MAXWELL_DMA_COPY_A),
        (0x400, 2),
        (0x404, 0),
        (0x408, 2),
        (0x40c, 60),
        (0x418, 4),
        (0x300, LINEAR_DMA_LAUNCH),
    ]);
    let stats = r.prepare(&m).unwrap().commit().1;
    assert_eq!((stats.clear_launches, stats.copy_launches), (2, 1));
    let p = r.pixels();
    assert_eq!(&p[..4], &[255, 255, 0, 255]);
    assert_eq!(&p[60..], &p[..4]);
    assert_eq!(&p[4..8], &[255, 0, 0, 255]);
}
#[test]
fn late_hole_and_shared_execution_budget_cancel_every_planned_row() {
    let mut r = Rig::new();
    let mut m = clear();
    m.extend([(0x804, 60), (0x19d0, 0x3c)]);
    assert!(matches!(r.prepare(&m), Err(ExecutionError::Memory(_))));
    assert_eq!(r.pixels(), [0x55; 64]);
    let mut m = clear();
    m.extend(std::iter::repeat_n((0x19d0, 0x3c), 64));
    assert!(matches!(r.prepare(&m), Err(ExecutionError::Capacity)));
    assert_eq!(r.pixels(), [0x55; 64]);
}
#[test]
fn render_write_ranges_exclude_scissored_bytes_and_padding() {
    let mut r = Rig::new();
    let mut m = clear();
    // Replace the full clear by a one-pixel rectangular clear.
    m.pop();
    m.extend([
        (0xd6c, (2 << 16) | 1),
        (0xd70, (2 << 16) | 1),
        (0x10f8, 0x10),
        (0x19d0, 0x3c),
    ]);
    let work = r.prepare(&m).unwrap();
    assert_eq!(work.write_ranges().map(|v| v.length).sum::<usize>(), 4);
    assert_eq!(work.color_clears()[0].rect, [1, 1, 2, 2]);
    work.commit();
    let mut expected = [0x55; 64];
    expected[20..24].copy_from_slice(&[255, 0, 0, 255]);
    assert_eq!(r.pixels(), expected);
}
