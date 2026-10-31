use emulator_gpu::*;
use emulator_memory::{Access, AddressSpace, Permissions};
const COMMAND: u64 = 0x1_0000_1000;
const DATA: u64 = 0x2_0000_0000;
const ALIAS: u64 = 0x4_0000_0000;
struct Rig {
    cpu: AddressSpace,
    gpu: GpuAddressSpace,
    queue: SubmissionQueue,
}
impl Rig {
    fn new() -> Self {
        let mut cpu = AddressSpace::with_limit(8192);
        cpu.map(0x1000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        cpu.map(0x3000, (0..64).collect(), Permissions::READ_WRITE)
            .unwrap();
        let mut gpu = GpuAddressSpace::new(8192).unwrap();
        gpu.map(
            GpuVa(COMMAND),
            cpu.pin(0x1000, 4096, Permissions::READ).unwrap(),
        )
        .unwrap();
        gpu.map(
            GpuVa(DATA),
            cpu.pin(0x3000, 64, Permissions::READ_WRITE).unwrap(),
        )
        .unwrap();
        let queue = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
        Self { cpu, gpu, queue }
    }
    fn submit(&mut self, methods: &[(u8, u32, u32)]) -> SubmissionId {
        let words: Vec<u32> = methods
            .iter()
            .flat_map(|&(sub, offset, data)| {
                [0x2001_0000 | (u32::from(sub) << 13) | (offset / 4), data]
            })
            .collect();
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.cpu.write(0x1000, &bytes).unwrap();
        let entry: Vec<u8> = [
            COMMAND as u32,
            ((COMMAND >> 32) as u32) | ((words.len() as u32) << 10),
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
        self.queue.submit(&self.gpu, &entry).unwrap()
    }
    fn execute(&mut self, id: SubmissionId) -> Result<ExecutionStats, ExecutionError> {
        self.queue
            .execute_front(&self.gpu, id, ExecutionLimits::default())
    }
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0; 64];
        self.cpu.read(0x3000, &mut bytes).unwrap();
        bytes
    }
}
fn registers(sub: u8, source: u64, target: u64, length: u32) -> Vec<(u8, u32, u32)> {
    vec![
        (sub, 0, MAXWELL_DMA_COPY_A),
        (sub, 0x400, (source >> 32) as u32),
        (sub, 0x404, source as u32),
        (sub, 0x408, (target >> 32) as u32),
        (sub, 0x40c, target as u32),
        (sub, 0x418, length),
    ]
}
fn copy(sub: u8, source: u64, target: u64, length: u32) -> Vec<(u8, u32, u32)> {
    let mut methods = registers(sub, source, target, length);
    methods.push((sub, 0x300, LINEAR_DMA_LAUNCH));
    methods
}
#[test]
fn real_copy_is_cpu_visible_and_consumed_once_with_execution_time_data() {
    let mut rig = Rig::new();
    let id = rig.submit(&copy(7, DATA, DATA + 16, 8));
    // Commands are immutable snapshots; data is resolved/read when executing.
    rig.cpu.write(0x1000, &[0xff; 56]).unwrap();
    rig.cpu.write(0x3000, &[99; 8]).unwrap();
    assert_eq!(
        rig.execute(id).unwrap(),
        ExecutionStats {
            methods: 7,
            copy_launches: 1,
            copied_bytes: 8
        }
    );
    assert_eq!(&rig.bytes()[16..24], &[99; 8]);
    assert_eq!(rig.queue.stats(), QueueStats::default());
    assert_eq!(rig.execute(id), Err(ExecutionError::NotFront));
}
#[test]
fn dependent_launches_observe_prior_writes_through_aliases() {
    let mut rig = Rig::new();
    rig.gpu
        .map(
            GpuVa(ALIAS),
            rig.cpu.pin(0x3000, 64, Permissions::READ_WRITE).unwrap(),
        )
        .unwrap();
    let mut methods = copy(0, DATA, DATA + 16, 8);
    methods.extend(copy(1, ALIAS + 16, DATA + 32, 8));
    let id = rig.submit(&methods);
    let stats = rig.execute(id).unwrap();
    assert_eq!((stats.copy_launches, stats.copied_bytes), (2, 16));
    assert_eq!(&rig.bytes()[32..40], &(0..8).collect::<Vec<_>>());
}
#[test]
fn subchannel_registers_persist_independently_and_rebind_resets() {
    let mut rig = Rig::new();
    let mut methods = registers(0, DATA, DATA + 16, 4);
    methods.extend(registers(1, DATA + 4, DATA + 32, 8));
    let id = rig.submit(&methods);
    assert_eq!(rig.execute(id).unwrap().copy_launches, 0);
    let id = rig.submit(&[(0, 0x300, LINEAR_DMA_LAUNCH), (1, 0x300, LINEAR_DMA_LAUNCH)]);
    assert_eq!(rig.execute(id).unwrap().copied_bytes, 12);
    assert_eq!(&rig.bytes()[16..20], &[0, 1, 2, 3]);
    assert_eq!(&rig.bytes()[32..40], &[4, 5, 6, 7, 8, 9, 10, 11]);
    let id = rig.submit(&[(0, 0, MAXWELL_DMA_COPY_A)]);
    rig.execute(id).unwrap();
    let id = rig.submit(&[(0, 0x300, LINEAR_DMA_LAUNCH)]);
    assert_eq!(rig.execute(id), Err(ExecutionError::UninitializedRegister));
}
#[test]
fn late_error_rolls_back_copies_binding_and_queue() {
    let mut rig = Rig::new();
    let mut methods = copy(0, DATA, DATA + 16, 8);
    methods.push((0, 0x900, 0));
    let id = rig.submit(&methods);
    let before = (rig.bytes(), rig.queue.stats());
    assert!(matches!(
        rig.execute(id),
        Err(ExecutionError::UnsupportedMethod { offset: 0x900, .. })
    ));
    assert_eq!((rig.bytes(), rig.queue.stats()), before);
    assert!(rig.execute(id).is_err());
    rig.queue.release_front(id).unwrap();
    let id = rig.submit(&[(0, 0x300, LINEAR_DMA_LAUNCH)]);
    assert_eq!(rig.execute(id), Err(ExecutionError::UnboundSubchannel(0)));
}
#[test]
fn unsupported_classes_methods_launch_flags_and_uninitialized_data_are_errors() {
    for (methods, expected) in [
        (
            vec![(0, 0, 0xb197)],
            ExecutionError::UnsupportedClass(0xb197),
        ),
        (
            vec![(0, 0, 0x1_0000 | MAXWELL_DMA_COPY_A)],
            ExecutionError::UnsupportedClass(0x1_b0b5),
        ),
        (
            vec![(0, 0, MAXWELL_DMA_COPY_A), (0, 0x400, 256)],
            ExecutionError::InvalidAddressUpper(256),
        ),
        (copy(0, DATA, DATA + 16, 0), ExecutionError::EmptyCopy),
        (
            vec![(0, 0, MAXWELL_DMA_COPY_A), (0, 0x300, LINEAR_DMA_LAUNCH)],
            ExecutionError::UninitializedRegister,
        ),
        (
            vec![(0, 0, MAXWELL_DMA_COPY_A), (0, 0x410, 8)],
            ExecutionError::UnsupportedMethod {
                subchannel: 0,
                offset: 0x410,
            },
        ),
    ] {
        let mut rig = Rig::new();
        let id = rig.submit(&methods);
        let before = (rig.bytes(), rig.queue.stats());
        assert_eq!(rig.execute(id), Err(expected));
        assert_eq!((rig.bytes(), rig.queue.stats()), before);
    }
    // Reject each unsupported field, including flush, multiline, remap,
    // physical addressing and semaphore/interrupt/cache controls.
    for flags in [
        0,
        0x181,
        0x183,
        0x102,
        0x82,
        0x182 | 4,
        0x182 | 8,
        0x182 | 32,
        0x182 | 512,
        0x182 | 1024,
        0x182 | 2048,
        0x182 | 4096,
        0x182 | 8192,
        0x182 | (1 << 20),
        0x8000_0182,
    ] {
        let mut rig = Rig::new();
        let mut methods = copy(0, DATA, DATA + 16, 8);
        methods.last_mut().unwrap().2 = flags;
        let id = rig.submit(&methods);
        assert_eq!(
            rig.execute(id),
            Err(ExecutionError::UnsupportedLaunch(flags))
        );
        assert_eq!(rig.bytes(), (0..64).collect::<Vec<_>>());
    }
}
#[test]
fn permission_gaps_and_address_overflow_fail_before_any_write() {
    for (source, target, length, access) in [
        (DATA, ALIAS, 8, Some(Access::Write)),
        (ALIAS, DATA + 16, 8, Some(Access::Read)),
        (DATA, DATA + 60, 8, None),
        ((1 << 40) - 4, DATA + 16, 8, None),
    ] {
        let mut rig = Rig::new();
        if let Some(access) = access {
            let permissions = if access == Access::Write {
                Permissions::READ
            } else {
                Permissions::NONE
            };
            rig.gpu
                .map(GpuVa(ALIAS), rig.cpu.pin(0x3000, 8, permissions).unwrap())
                .unwrap();
        }
        let id = rig.submit(&copy(0, source, target, length));
        let before = (rig.bytes(), rig.queue.stats());
        assert!(matches!(rig.execute(id), Err(ExecutionError::Memory(_))));
        assert_eq!((rig.bytes(), rig.queue.stats()), before);
    }
}
#[test]
fn overlapping_backing_is_rejected_even_across_distinct_virtual_aliases() {
    let mut rig = Rig::new();
    rig.gpu
        .map(
            GpuVa(ALIAS),
            rig.cpu.pin(0x3000, 64, Permissions::READ_WRITE).unwrap(),
        )
        .unwrap();
    for target in [DATA, DATA + 4, ALIAS, ALIAS + 4] {
        let id = rig.submit(&copy(0, DATA, target, 8));
        assert_eq!(rig.execute(id), Err(ExecutionError::OverlappingCopy));
        assert_eq!(rig.bytes(), (0..64).collect::<Vec<_>>());
        rig.queue.release_front(id).unwrap();
    }
    let id = rig.submit(&copy(0, DATA, ALIAS + 8, 8));
    rig.execute(id).unwrap();
    assert_eq!(&rig.bytes()[8..16], &[0, 1, 2, 3, 4, 5, 6, 7]);
}
#[test]
fn limits_fifo_and_address_space_ownership_prevent_execution() {
    let mut rig = Rig::new();
    let mut methods = copy(0, DATA, DATA + 16, 8);
    methods.extend(copy(1, DATA, DATA + 32, 8));
    let first = rig.submit(&methods);
    let second = rig.submit(&copy(0, DATA, DATA + 16, 8));
    let stats = rig.queue.stats();
    assert_eq!(rig.execute(second), Err(ExecutionError::NotFront));
    let other = GpuAddressSpace::new(0).unwrap();
    assert_eq!(
        rig.queue
            .execute_front(&other, first, ExecutionLimits::default()),
        Err(ExecutionError::WrongAddressSpace)
    );
    for limits in [
        ExecutionLimits {
            bytes: 15,
            launches: 2,
        },
        ExecutionLimits {
            bytes: 16,
            launches: 1,
        },
    ] {
        assert_eq!(
            rig.queue.execute_front(&rig.gpu, first, limits),
            Err(ExecutionError::Capacity)
        );
        assert_eq!(rig.queue.stats(), stats);
        assert_eq!(rig.bytes(), (0..64).collect::<Vec<_>>());
    }
    for limits in [
        ExecutionLimits {
            bytes: 0,
            launches: 1,
        },
        ExecutionLimits {
            bytes: 1,
            launches: 65,
        },
        ExecutionLimits {
            bytes: 4 * 1024 * 1024 + 1,
            launches: 1,
        },
    ] {
        assert_eq!(
            rig.queue.execute_front(&rig.gpu, first, limits),
            Err(ExecutionError::InvalidLimits)
        );
    }
    assert_eq!(rig.execute(first).unwrap().copy_launches, 2);
    rig.execute(second).unwrap();
}
