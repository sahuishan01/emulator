use emulator_gpu::*;
use emulator_memory::{AddressSpace, Permissions};

const BASE: u64 = 1 << 32;
const SEGMENT_WORDS: u32 = 8 * 1024;

/// Three 8K-word segments (24K words) in one submit, like NVN's large
/// multi-segment submits; each segment is complete 2-word packets.
fn setup() -> (AddressSpace, GpuAddressSpace, Vec<u8>) {
    let words = SEGMENT_WORDS as usize * 3;
    let mut page = Vec::with_capacity(words * 4);
    for packet in 0..words / 2 {
        // Incrementing method, count 1, subchannel 0, method 0x40 / 4.
        page.extend_from_slice(&(0x2001_0010u32).to_le_bytes());
        page.extend_from_slice(&(packet as u32).to_le_bytes());
    }
    let mut cpu = AddressSpace::default();
    cpu.map(0x10000, page, Permissions::READ_WRITE).unwrap();
    let mut gpu = GpuAddressSpace::new(1 << 20).unwrap();
    gpu.map(
        GpuVa(BASE),
        cpu.pin(0x10000, words * 4, Permissions::READ_WRITE)
            .unwrap(),
    )
    .unwrap();
    let mut entries = Vec::new();
    for segment in 0..3u32 {
        let low = segment * SEGMENT_WORDS * 4;
        entries.extend_from_slice(&low.to_le_bytes());
        entries.extend_from_slice(&(1 | (SEGMENT_WORDS << 10)).to_le_bytes());
    }
    (cpu, gpu, entries)
}

#[test]
fn one_submit_may_capture_more_than_one_entry_limit_across_segments() {
    let (_cpu, gpu, entries) = setup();
    let mut queue = SubmissionQueue::new(&gpu, QueueLimits::default()).unwrap();
    queue.submit(&gpu, &entries).unwrap();
    let submission = queue.front().unwrap();
    assert_eq!(submission.writes().len(), SEGMENT_WORDS as usize * 3 / 2);
    assert_eq!(submission.writes()[5].data, 5);
}

#[test]
fn per_submit_word_cap_still_rejects_without_publishing() {
    let (_cpu, gpu, entries) = setup();
    let limits = QueueLimits {
        bytes: SEGMENT_WORDS as usize * 4 * 2,
        ..QueueLimits::default()
    };
    let mut queue = SubmissionQueue::new(&gpu, limits).unwrap();
    assert_eq!(queue.submit(&gpu, &entries), Err(QueueError::Capacity));
    assert!(queue.front().is_none());
}
