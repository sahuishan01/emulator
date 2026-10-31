use emulator_gpu::*;
use emulator_memory::{Access, AddressSpace, Permissions};
fn wire(address: u64, words: u32) -> Vec<u8> {
    [(address as u32), ((address >> 32) as u32) | (words << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}
fn setup() -> (AddressSpace, GpuAddressSpace, MappingId) {
    let mut cpu = AddressSpace::with_limit(64);
    cpu.map(
        0x1000,
        [0x2003_0040_u32, 10, 20, 30]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect(),
        Permissions::READ_WRITE,
    )
    .unwrap();
    let mut gpu = GpuAddressSpace::new(64).unwrap();
    let mapping = gpu
        .map(
            GpuVa(0x1_0000_1000),
            cpu.pin(0x1000, 16, Permissions::READ_WRITE).unwrap(),
        )
        .unwrap();
    (cpu, gpu, mapping)
}
#[test]
fn gpu_aliases_observe_cpu_writes_and_survive_unmap_and_address_reuse() {
    let (mut cpu, mut gpu, mapping) = setup();
    let address = GpuVa(0x1_0000_1000);
    cpu.write_u32(0x1004, 99).unwrap();
    let mut value = [0; 4];
    gpu.read(GpuVa(address.0 + 4), &mut value).unwrap();
    assert_eq!(u32::from_le_bytes(value), 99);
    gpu.write(GpuVa(address.0 + 4), &42_u32.to_le_bytes())
        .unwrap();
    assert_eq!(cpu.read_u32(0x1004).unwrap(), 42);
    let pinned = gpu.pin(address, 16, Access::Read).unwrap();
    let old = pinned.backing_ranges().next().unwrap().id;
    gpu.unmap(mapping).unwrap();
    cpu.unmap(0x1000, 16).unwrap();
    assert_eq!(cpu.resident_bytes(), 16);
    cpu.map(0x1000, vec![8; 16], Permissions::READ).unwrap();
    let replacement = gpu
        .map(address, cpu.pin(0x1000, 16, Permissions::READ).unwrap())
        .unwrap();
    assert!(gpu.unmap(mapping).is_err());
    gpu.read(address, &mut value).unwrap();
    assert_eq!(value, [8; 4]);
    let mut original = [0; 16];
    pinned.read(&mut original).unwrap();
    assert_eq!(u32::from_le_bytes(original[4..8].try_into().unwrap()), 42);
    assert_ne!(
        gpu.pin(address, 16, Access::Read)
            .unwrap()
            .backing_ranges()
            .next()
            .unwrap()
            .id,
        old
    );
    gpu.unmap(replacement).unwrap();
    cpu.unmap(0x1000, 16).unwrap();
    assert_eq!(cpu.resident_bytes(), 16);
    drop(pinned);
    assert_eq!(cpu.resident_bytes(), 0);
}
#[test]
fn cross_mapping_gpu_access_is_atomic_and_mapping_limits_fail_without_changes() {
    let (mut cpu, mut gpu, _) = setup();
    cpu.map(0x2000, vec![7; 4], Permissions::READ).unwrap();
    gpu.map(
        GpuVa(0x1_0000_1010),
        cpu.pin(0x2000, 4, Permissions::READ).unwrap(),
    )
    .unwrap();
    assert!(gpu.write(GpuVa(0x1_0000_100e), &[0; 4]).is_err());
    assert_eq!(cpu.read_u32(0x100c).unwrap(), 30);
    let mut data = [0xaa; 4];
    assert!(gpu.read(GpuVa(0x1_0000_1012), &mut data).is_err());
    assert_eq!(data, [0xaa; 4]);
    let before = gpu.mapped_bytes();
    assert!(gpu
        .map(
            GpuVa(0x1_0000_1000),
            cpu.pin(0x2000, 4, Permissions::READ).unwrap()
        )
        .is_err());
    assert_eq!(gpu.mapped_bytes(), before);
    assert!(gpu
        .map(
            GpuVa((1 << 40) - 2),
            cpu.pin(0x2000, 4, Permissions::READ).unwrap()
        )
        .is_err());
    let mut tiny = GpuAddressSpace::new(2).unwrap();
    assert!(tiny
        .map(GpuVa(0), cpu.pin(0x2000, 4, Permissions::READ).unwrap())
        .is_err());
    assert_eq!(tiny.mapping_count(), 0);
    assert!(gpu.pin(GpuVa(0x1_0000_1000), 4, Access::Execute).is_err());
}
#[test]
fn queued_packets_cross_entries_and_own_immutable_commands_and_original_backing() {
    let (mut cpu, mut gpu, mapping) = setup();
    let address = 0x1_0000_1000;
    let mut queue = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
    let mut entries = wire(address, 2);
    entries.extend(wire(address + 8, 2));
    let first = queue.submit(&gpu, &entries).unwrap();
    assert_eq!(
        queue
            .front()
            .unwrap()
            .writes()
            .iter()
            .map(|w| w.data)
            .collect::<Vec<_>>(),
        [10, 20, 30]
    );
    let old = queue.front().unwrap().source_ranges().next().unwrap().id;
    cpu.write_u32(0x1004, 99).unwrap();
    assert_eq!(queue.front().unwrap().writes()[0].data, 10);
    cpu.unmap(0x1000, 16).unwrap();
    gpu.unmap(mapping).unwrap();
    assert_eq!(cpu.resident_bytes(), 16);
    cpu.map(
        0x1000,
        0x807b_0044_u32.to_le_bytes().to_vec(),
        Permissions::READ,
    )
    .unwrap();
    let replacement = gpu
        .map(
            GpuVa(address),
            cpu.pin(0x1000, 4, Permissions::READ).unwrap(),
        )
        .unwrap();
    assert_ne!(
        gpu.pin(GpuVa(address), 4, Access::Read)
            .unwrap()
            .backing_ranges()
            .next()
            .unwrap()
            .id,
        old
    );
    let second = queue.submit(&gpu, &wire(address, 1)).unwrap();
    assert_ne!(first, second);
    assert!(queue.release_front(second).is_err());
    assert_eq!(queue.stats().pending, 2);
    assert_eq!(cpu.resident_bytes(), 20);
    queue.release_front(first).unwrap();
    assert_eq!(cpu.resident_bytes(), 4);
    assert_eq!(queue.front().unwrap().writes()[0].data, 123);
    cpu.unmap(0x1000, 4).unwrap();
    gpu.unmap(replacement).unwrap();
    assert_eq!(cpu.resident_bytes(), 4);
    queue.release_front(second).unwrap();
    assert_eq!(cpu.resident_bytes(), 0);
    assert_eq!(queue.stats(), QueueStats::default());
}
#[test]
fn failed_submissions_and_limits_publish_nothing_and_queues_enforce_ownership() {
    let (mut cpu, gpu, _) = setup();
    let address = 0x1_0000_1000;
    let mut queue = SubmissionQueue::new(
        &gpu,
        QueueLimits {
            pending: 1,
            bytes: 16,
            writes: 3,
        },
    )
    .unwrap();
    let before = queue.stats();
    let mut bad = wire(address, 4);
    bad.extend(wire(address + 0x1000, 1));
    assert!(queue.submit(&gpu, &bad).is_err());
    assert_eq!(queue.stats(), before);
    let mut unmapped = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
    assert!(matches!(
        unmapped.submit(&gpu, &bad),
        Err(QueueError::Memory(GpuMemoryError::Unmapped(_)))
    ));
    assert_eq!(unmapped.stats(), QueueStats::default());
    // A late unsupported packet rejects the entire capture, including prior writes.
    cpu.write_u32(0x1000, 0x8001_0000).unwrap();
    assert!(queue.submit(&gpu, &wire(address, 4)).is_err());
    assert_eq!(queue.stats(), before);
    cpu.write_u32(0x1000, 0x2003_0040).unwrap();
    let id = queue.submit(&gpu, &wire(address, 4)).unwrap();
    let full = queue.stats();
    assert!(queue.submit(&gpu, &wire(address, 4)).is_err());
    assert_eq!(queue.stats(), full);
    let other = GpuAddressSpace::new(64).unwrap();
    assert_eq!(
        queue.submit(&other, &wire(address, 4)),
        Err(QueueError::WrongAddressSpace)
    );
    let mut second = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
    let second_id = second.submit(&gpu, &wire(address, 4)).unwrap();
    assert_ne!(id, second_id);
    assert_eq!(second.release_front(id), Err(QueueError::NotFront));
    let mut limited = SubmissionQueue::new(
        &gpu,
        QueueLimits {
            pending: 1,
            bytes: 8,
            writes: 3,
        },
    )
    .unwrap();
    assert!(limited.submit(&gpu, &wire(address, 4)).is_err());
    assert_eq!(limited.stats(), QueueStats::default());
    let mut limited = SubmissionQueue::new(
        &gpu,
        QueueLimits {
            pending: 1,
            bytes: 16,
            writes: 2,
        },
    )
    .unwrap();
    assert!(limited.submit(&gpu, &wire(address, 4)).is_err());
    assert_eq!(limited.stats(), QueueStats::default());
}
#[test]
fn queued_pins_cannot_bypass_the_originating_cpu_resident_limit() {
    let mut cpu = AddressSpace::with_limit(4);
    cpu.map(0, 0x8001_0000_u32.to_le_bytes().to_vec(), Permissions::READ)
        .unwrap();
    let mut gpu = GpuAddressSpace::new(4).unwrap();
    let mapping = gpu
        .map(GpuVa(0), cpu.pin(0, 4, Permissions::READ).unwrap())
        .unwrap();
    let mut queue = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
    let id = queue.submit(&gpu, &wire(0, 1)).unwrap();
    cpu.unmap(0, 4).unwrap();
    gpu.unmap(mapping).unwrap();
    assert!(cpu.map(0, vec![0; 4], Permissions::READ).is_err());
    queue.release_front(id).unwrap();
    cpu.map(0, vec![0; 4], Permissions::READ).unwrap();
}

#[test]
fn speculative_mapping_snapshots_isolate_tables_and_never_reuse_canceled_ids() {
    let mut memory = AddressSpace::default();
    memory
        .map(0x1000, vec![42; 4096], Permissions::READ_WRITE)
        .unwrap();
    let view = memory.pin(0x1000, 4096, Permissions::READ_WRITE).unwrap();
    let mut live = GpuAddressSpace::new(16384).unwrap();
    let original = live.map(GpuVa(0x10000), view.clone()).unwrap();
    let mut canceled = live.snapshot_for_transaction();
    assert_eq!(canceled.id(), live.id());
    canceled.unmap(original).unwrap();
    let discarded = canceled.map(GpuVa(0x20000), view.clone()).unwrap();
    assert_eq!(live.mapping_count(), 1);
    let mut value = [0; 1];
    live.read(GpuVa(0x10000), &mut value).unwrap();
    assert_eq!(value, [42]);
    assert!(live.read(GpuVa(0x20000), &mut value).is_err());
    drop(canceled);
    let mut candidate = live.snapshot_for_transaction();
    let published = candidate.map(GpuVa(0x20000), view).unwrap();
    assert_ne!(discarded, published);
    assert!(candidate.unmap(discarded).is_err());
    live = candidate;
    assert_eq!(live.mapping_count(), 2);
    assert_eq!(live.mapped_spans(), 2);
    live.unmap(original).unwrap();
    live.read(GpuVa(0x20000), &mut value).unwrap();
    assert_eq!(value, [42]);
}
