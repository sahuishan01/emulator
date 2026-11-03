use emulator_gpu::*;
use emulator_memory::{Access, AddressSpace, Permissions};

const COMMAND: u64 = 1 << 32;
const DATA: u64 = 2 << 32;
const ALIAS: u64 = 3 << 32;
const SPLIT: u64 = 4 << 32;

struct Rig {
    cpu: AddressSpace,
    gpu: GpuAddressSpace,
}
impl Rig {
    fn new() -> Self {
        let mut cpu = AddressSpace::default();
        cpu.map(0x1000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        cpu.map(0x3000, (0..128).collect(), Permissions::READ_WRITE)
            .unwrap();
        let mut gpu = GpuAddressSpace::new(16384).unwrap();
        gpu.map(
            GpuVa(COMMAND),
            cpu.pin(0x1000, 4096, Permissions::READ).unwrap(),
        )
        .unwrap();
        for address in [DATA, ALIAS] {
            gpu.map(
                GpuVa(address),
                cpu.pin(0x3000, 128, Permissions::READ_WRITE).unwrap(),
            )
            .unwrap();
        }
        Self { cpu, gpu }
    }
    fn prepare(
        &mut self,
        methods: &[(u32, u32)],
        limits: ExecutionLimits,
    ) -> Result<PreparedSubmission, ExecutionError> {
        self.prepare_deferred(methods, limits)?.snapshot_inputs()
    }
    fn prepare_deferred(
        &mut self,
        methods: &[(u32, u32)],
        limits: ExecutionLimits,
    ) -> Result<PreparedSubmission, ExecutionError> {
        let bytes: Vec<u8> = methods
            .iter()
            .flat_map(|(offset, data)| [0x2001_0000 | (offset / 4), *data])
            .flat_map(u32::to_le_bytes)
            .collect();
        self.cpu.write(0x1000, &bytes).unwrap();
        let entries: Vec<u8> = [0, 1 | ((bytes.len() as u32 / 4) << 10)]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let mut queue = SubmissionQueue::new(&self.gpu, QueueLimits::default()).unwrap();
        let id = queue.submit(&self.gpu, &entries).unwrap();
        queue.prepare_front(&self.gpu, id, limits)
    }
    fn bytes(&self) -> [u8; 128] {
        let mut bytes = [0; 128];
        self.cpu.read(0x3000, &mut bytes).unwrap();
        bytes
    }
    fn read_pin(&self, address: u64, length: usize) -> GpuMemoryRange {
        self.gpu.pin(GpuVa(address), length, Access::Read).unwrap()
    }
}
fn dma(source: u64, target: u64, length: u32) -> Vec<(u32, u32)> {
    vec![
        (0, MAXWELL_DMA_COPY_A),
        (0x400, (source >> 32) as u32),
        (0x404, source as u32),
        (0x408, (target >> 32) as u32),
        (0x40c, target as u32),
        (0x418, length),
        (0x300, LINEAR_DMA_LAUNCH),
    ]
}
fn clear(target: u64, mask: u32) -> Vec<(u32, u32)> {
    vec![
        (0, MAXWELL_B),
        (0x800, (target >> 32) as u32),
        (0x804, target as u32),
        (0x808, 4),
        (0x80c, 1),
        (0x810, RGBA8_UNORM),
        (0x814, 0x1000),
        (0x818, 1),
        (0x81c, 0),
        (0x820, 0),
        (0xff4, 1 << 16),
        (0xff8, 1 << 16),
        (0x10f8, 0),
        (0xd80, 0),
        (0xd84, 1f32.to_bits()),
        (0xd88, 0),
        (0xd8c, 1f32.to_bits()),
        (0x19d0, mask),
    ]
}

#[test]
fn ordered_alias_projection_is_repeatable_cancelable_and_matches_publication() {
    let mut r = Rig::new();
    let pin = r.read_pin(ALIAS, 128);
    let mut methods = dma(DATA, DATA + 16, 8);
    methods.extend(dma(ALIAS + 16, DATA + 32, 8));
    let before = r.bytes();
    let work = r.prepare(&methods, ExecutionLimits::default()).unwrap();
    let mut expected = before;
    expected[16..24].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7]);
    expected[32..40].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7]);
    for _ in 0..3 {
        let mut projected = [0; 128];
        work.read_after(&pin, &mut projected).unwrap();
        assert_eq!(projected, expected);
        assert_eq!(r.bytes(), before);
    }
    drop(work);
    assert_eq!(r.bytes(), before);
    let work = r.prepare(&methods, ExecutionLimits::default()).unwrap();
    assert_eq!(work.commit().1.copied_bytes, 16);
    assert_eq!(r.bytes(), expected);
}

#[test]
fn prepared_dma_bytes_are_immutable_but_unwritten_preview_bytes_remain_live() {
    let mut r = Rig::new();
    let pin = r.read_pin(ALIAS + 16, 16);
    let work = r
        .prepare(&dma(DATA, DATA + 16, 8), ExecutionLimits::default())
        .unwrap();
    r.cpu.write(0x3000, &[99; 8]).unwrap();
    r.cpu.write(0x3010, &[88; 8]).unwrap();
    r.cpu.write(0x3018, &[77; 8]).unwrap();
    let mut projected = [0; 16];
    work.read_after(&pin, &mut projected).unwrap();
    assert_eq!(
        projected,
        [0, 1, 2, 3, 4, 5, 6, 7, 77, 77, 77, 77, 77, 77, 77, 77]
    );
    work.commit();
    assert_eq!(&r.bytes()[16..32], &projected);
    assert_eq!(&r.bytes()[..8], &[99; 8]);
}

#[test]
fn masked_clear_reads_earlier_dma_and_later_dma_reads_the_merged_color() {
    let mut r = Rig::new();
    r.cpu.write(0x3000, &[5, 9, 13, 17]).unwrap();
    let mut methods = dma(DATA, DATA + 16, 4);
    methods.extend(clear(ALIAS + 16, 8)); // Only green is written.
    methods.extend(dma(DATA + 16, DATA + 32, 4));
    let work = r.prepare(&methods, ExecutionLimits::default()).unwrap();
    let pin = r.read_pin(DATA, 128);
    let mut projected = [0; 128];
    work.read_after(&pin, &mut projected).unwrap();
    assert_eq!(&projected[16..20], &[5, 255, 13, 17]);
    assert_eq!(&projected[32..36], &[5, 255, 13, 17]);
    r.cpu.write(0x3010, &[66; 4]).unwrap();
    work.commit();
    assert_eq!(r.bytes(), projected);
}

#[test]
fn split_and_reordered_target_spans_use_backing_offsets() {
    let mut r = Rig::new();
    for (gpu, cpu) in [(SPLIT, 0x3010), (SPLIT + 4, 0x300c)] {
        r.gpu
            .map(
                GpuVa(gpu),
                r.cpu.pin(cpu, 4, Permissions::READ_WRITE).unwrap(),
            )
            .unwrap();
    }
    let work = r
        .prepare(&dma(DATA, SPLIT, 8), ExecutionLimits::default())
        .unwrap();
    let mut projected = [0; 16];
    work.read_after(&r.read_pin(ALIAS + 10, 16), &mut projected)
        .unwrap();
    assert_eq!(
        projected,
        [10, 11, 4, 5, 6, 7, 0, 1, 2, 3, 20, 21, 22, 23, 24, 25]
    );
    let mut logical = [0; 8];
    work.read_after(&r.read_pin(SPLIT, 8), &mut logical)
        .unwrap();
    assert_eq!(logical, [0, 1, 2, 3, 4, 5, 6, 7]);
    work.commit();
    assert_eq!(&r.bytes()[10..26], &projected);
}

#[test]
fn repeated_backing_inside_one_target_pin_obeys_span_write_order() {
    let mut r = Rig::new();
    for address in [SPLIT, SPLIT + 4] {
        r.gpu
            .map(
                GpuVa(address),
                r.cpu.pin(0x3010, 4, Permissions::READ_WRITE).unwrap(),
            )
            .unwrap();
    }
    let work = r
        .prepare(&dma(DATA, SPLIT, 8), ExecutionLimits::default())
        .unwrap();
    let pin = r.read_pin(SPLIT, 8);
    let mut projected = [0; 8];
    work.read_after(&pin, &mut projected).unwrap();
    assert_eq!(projected, [4, 5, 6, 7, 4, 5, 6, 7]);
    work.commit();
    let mut actual = [0; 8];
    pin.read(&mut actual).unwrap();
    assert_eq!(actual, projected);
}

#[test]
fn partially_overlapping_effects_and_chained_reads_take_the_last_ordered_writer() {
    let mut r = Rig::new();
    let mut methods = dma(DATA, DATA + 16, 8);
    methods.extend(dma(DATA + 8, DATA + 20, 8));
    methods.extend(dma(ALIAS + 18, DATA + 40, 8));
    let work = r.prepare(&methods, ExecutionLimits::default()).unwrap();
    let mut projected = [0; 128];
    work.read_after(&r.read_pin(DATA, 128), &mut projected)
        .unwrap();
    assert_eq!(
        &projected[16..28],
        &[0, 1, 2, 3, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    assert_eq!(&projected[40..48], &[2, 3, 8, 9, 10, 11, 12, 13]);
    work.commit();
    assert_eq!(r.bytes(), projected);
}

#[test]
fn projection_rejects_write_pins_wrong_sizes_and_oversized_outputs_without_writing() {
    let mut r = Rig::new();
    let work = r
        .prepare(&dma(DATA, DATA + 16, 8), ExecutionLimits::default())
        .unwrap();
    let write_only = r.gpu.pin(GpuVa(DATA + 16), 8, Access::Write).unwrap();
    let mut output = [0xa5; 8];
    assert!(matches!(
        work.read_after(&write_only, &mut output),
        Err(ExecutionError::Memory(GpuMemoryError::PermissionDenied(
            Access::Read
        )))
    ));
    assert_eq!(output, [0xa5; 8]);
    let read = r.read_pin(DATA + 16, 16);
    assert!(matches!(
        work.read_after(&read, &mut output),
        Err(ExecutionError::Memory(_))
    ));
    assert_eq!(output, [0xa5; 8]);
    assert_eq!(
        work.read_after(&read, &mut vec![0; 4 * 1024 * 1024 + 1]),
        Err(ExecutionError::Capacity)
    );
    assert_eq!(r.bytes(), std::array::from_fn(|i| i as u8));
}

#[test]
fn pins_keep_old_backing_when_cpu_and_gpu_addresses_are_reused() {
    let mut r = Rig::new();
    let pin = r.read_pin(ALIAS + 16, 8);
    let work = r
        .prepare(&dma(DATA, DATA + 16, 8), ExecutionLimits::default())
        .unwrap();
    for address in [DATA, ALIAS] {
        let id = r
            .gpu
            .pin(GpuVa(address), 128, Access::Read)
            .unwrap()
            .mapping_ids()
            .next()
            .unwrap();
        r.gpu.unmap(id).unwrap();
    }
    r.cpu.unmap(0x3000, 128).unwrap();
    r.cpu
        .map(0x3000, vec![99; 128], Permissions::READ_WRITE)
        .unwrap();
    r.gpu
        .map(
            GpuVa(DATA),
            r.cpu.pin(0x3000, 128, Permissions::READ_WRITE).unwrap(),
        )
        .unwrap();
    let mut projected = [0; 8];
    work.read_after(&pin, &mut projected).unwrap();
    assert_eq!(projected, [0, 1, 2, 3, 4, 5, 6, 7]);
    work.commit();
    let mut actual = [0; 8];
    pin.read(&mut actual).unwrap();
    assert_eq!(actual, projected);
    assert_eq!(r.bytes(), [99; 128]);
    assert_eq!(r.cpu.resident_bytes(), 4096 + 256);
    drop(pin);
    assert_eq!(r.cpu.resident_bytes(), 4096 + 128);
}

#[test]
fn late_failure_and_exact_byte_budget_preserve_transactional_preparation() {
    let mut r = Rig::new();
    let before = r.bytes();
    let mut methods = dma(DATA, DATA + 16, 8);
    methods.extend(dma(ALIAS + 16, DATA + 32, 8));
    let mut bad = methods.clone();
    bad.push((0x900, 0));
    assert!(r.prepare(&bad, ExecutionLimits::default()).is_err());
    assert_eq!(r.bytes(), before);
    assert!(matches!(
        r.prepare(
            &methods,
            ExecutionLimits {
                bytes: 15,
                launches: 2
            }
        ),
        Err(ExecutionError::Capacity)
    ));
    assert_eq!(r.bytes(), before);
    let work = r
        .prepare(
            &methods,
            ExecutionLimits {
                bytes: 16,
                launches: 2,
            },
        )
        .unwrap();
    assert_eq!(work.stats().copied_bytes, 16);
    let mut projected = [0; 128];
    work.read_after(&r.read_pin(DATA, 128), &mut projected)
        .unwrap();
    work.commit();
    assert_eq!(r.bytes(), projected);
}

#[test]
fn full_clear_preserves_write_only_rights_and_masked_clear_requires_read_rights() {
    let mut r = Rig::new();
    r.gpu
        .map(
            GpuVa(SPLIT),
            r.cpu.pin(0x3010, 4, Permissions::WRITE).unwrap(),
        )
        .unwrap();
    let before = r.bytes();
    assert!(r
        .prepare(&clear(SPLIT, 8), ExecutionLimits::default())
        .is_err());
    assert_eq!(r.bytes(), before);
    let work = r
        .prepare(&clear(SPLIT, 0x3c), ExecutionLimits::default())
        .unwrap();
    let mut projected = [0; 4];
    work.read_after(&r.read_pin(ALIAS + 16, 4), &mut projected)
        .unwrap();
    assert_eq!(projected, [0, 255, 0, 255]);
    work.commit();
    assert_eq!(&r.bytes()[16..20], &projected);
}

#[test]
fn snapshots_are_explicit_and_idempotent_while_the_default_commit_reads_live_data() {
    let mut r = Rig::new();
    let methods = dma(DATA, DATA + 16, 8);
    let pin = r.read_pin(DATA + 16, 8);
    let work = r
        .prepare_deferred(&methods, ExecutionLimits::default())
        .unwrap();
    let mut output = [0xa5; 8];
    assert_eq!(
        work.read_after(&pin, &mut output),
        Err(ExecutionError::InputsNotSnapshotted)
    );
    assert_eq!(output, [0xa5; 8]);
    r.cpu.write(0x3000, &[50; 8]).unwrap();
    let work = work.snapshot_inputs().unwrap();
    r.cpu.write(0x3000, &[90; 8]).unwrap();
    let work = work.snapshot_inputs().unwrap();
    work.read_after(&pin, &mut output).unwrap();
    assert_eq!(output, [50; 8]);
    work.commit();
    assert_eq!(&r.bytes()[16..24], &[50; 8]);
    let work = r
        .prepare_deferred(&methods, ExecutionLimits::default())
        .unwrap();
    r.cpu.write(0x3000, &[70; 8]).unwrap();
    work.commit();
    assert_eq!(&r.bytes()[16..24], &[70; 8]);
}

#[test]
fn mixed_transactions_match_a_scalar_byte_oracle_and_the_default_executor() {
    // Deterministic independent byte operations: no backing-intersection or
    // projection helper is used to compute this oracle. Alternate alias VAs.
    let mut seed = 0xcab0_1234u32;
    for _ in 0..128 {
        let mut expected: [u8; 128] = std::array::from_fn(|i| i as u8);
        let mut methods = Vec::new();
        for index in 0..12 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let base = if seed & 1 == 0 { DATA } else { ALIAS };
            if index % 3 == 0 {
                let offset = (seed >> 16) as usize % 32 * 4;
                let mask = if seed & 2 == 0 { 8 } else { 0x3c };
                methods.extend(clear(base + offset as u64, mask));
                if mask == 8 {
                    expected[offset + 1] = 255;
                } else {
                    expected[offset..offset + 4].copy_from_slice(&[0, 255, 0, 255]);
                }
            } else {
                let half = if seed & 4 == 0 { 0 } else { 64 };
                let source = half + (seed >> 8) as usize % 15 * 4;
                let target = 64 - half + (seed >> 20) as usize % 15 * 4;
                methods.extend(dma(base + source as u64, base + target as u64, 8));
                expected.copy_within(source..source + 8, target);
            }
        }
        for snapshot in [false, true] {
            let mut r = Rig::new();
            let work = r
                .prepare_deferred(&methods, ExecutionLimits::default())
                .unwrap();
            let work = if snapshot {
                work.snapshot_inputs().unwrap()
            } else {
                work
            };
            if snapshot {
                let mut projected = [0; 128];
                work.read_after(&r.read_pin(ALIAS, 128), &mut projected)
                    .unwrap();
                assert_eq!(projected, expected);
            }
            assert_eq!(r.bytes(), std::array::from_fn(|i| i as u8));
            work.commit();
            assert_eq!(r.bytes(), expected);
        }
    }
}
