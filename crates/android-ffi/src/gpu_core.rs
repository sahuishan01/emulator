//! Actual CPU/GPU backing, command capture and original DMA method checks.
//! Includes owned sampled-draw reference cases; native Vulkan is checked separately.
use emulator_gpu::{
    ExecutionLimits, GpuAddressSpace, GpuVa, QueueLimits, QueueStats, SubmissionQueue,
    LINEAR_DMA_LAUNCH, MAXWELL_DMA_COPY_A,
};
use emulator_memory::{AddressSpace, Permissions};
type Error = Box<dyn std::error::Error>;
type Metrics = (usize, usize, usize, usize, usize);
type Check = fn() -> Result<Metrics, Error>;
fn require(value: bool, message: &str) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn wire(address: u64, words: u32) -> Vec<u8> {
    [address as u32, ((address >> 32) as u32) | (words << 10)]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}

fn aliases() -> Result<Metrics, Error> {
    let mut cpu = AddressSpace::with_limit(64);
    cpu.map(0x1000, vec![0; 8], Permissions::READ_WRITE)?;
    {
        let view = cpu.pin(0x1000, 8, Permissions::READ_WRITE)?;
        cpu.map_alias(0x2000, &view, Permissions::READ_WRITE)?;
    }
    let mut gpu = GpuAddressSpace::new(64)?;
    let address = GpuVa(0x1_0000_1000);
    let id = gpu.map(address, cpu.pin(0x1000, 8, Permissions::READ_WRITE)?)?;
    cpu.write_u64(0x2000, 42)?;
    let mut data = [0; 8];
    gpu.read(address, &mut data)?;
    require(
        u64::from_le_bytes(data) == 42,
        "CPU alias write did not reach GPU mapping",
    )?;
    gpu.write(address, &[7; 8])?;
    require(
        cpu.read_u64(0x1000)? == u64::from_le_bytes([7; 8]),
        "GPU write did not reach CPU alias",
    )?;
    cpu.protect(0x1000, 8, Permissions::READ)?;
    require(cpu.write_u64(0x1000, 0).is_err(), "CPU protection failed")?;
    require(
        cpu.pin(0x1000, 8, Permissions::READ_WRITE).is_err(),
        "A new pin widened protected permissions",
    )?;
    gpu.write(address, &[6; 8])?;
    require(
        cpu.read_u64(0x2000)? == u64::from_le_bytes([6; 8]),
        "CPU protection detached existing backing",
    )?;
    cpu.unmap(0x1000, 8)?;
    cpu.unmap(0x2000, 8)?;
    require(
        cpu.mapped_bytes() == 0 && cpu.resident_bytes() == 8,
        "GPU ownership did not retain/account backing",
    )?;
    cpu.map(0x1000, vec![9; 8], Permissions::READ)?;
    gpu.read(address, &mut data)?;
    require(
        data == [6; 8] && cpu.read_u64(0x1000)? == u64::from_le_bytes([9; 8]),
        "Address reuse redirected a GPU mapping",
    )?;
    gpu.unmap(id)?;
    cpu.unmap(0x1000, 8)?;
    require(
        cpu.resident_bytes() == 0,
        "Released alias backing remained resident",
    )?;
    Ok((0, 0, cpu.resident_bytes(), 0, 0))
}
fn submission() -> Result<Metrics, Error> {
    let mut cpu = AddressSpace::with_limit(64);
    cpu.map(
        0x1000,
        [0x2003_0040_u32, 10, 20, 30]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect(),
        Permissions::READ_WRITE,
    )?;
    let mut gpu = GpuAddressSpace::new(64)?;
    let address = 0x1_0000_1000;
    let id = gpu.map(
        GpuVa(address),
        cpu.pin(0x1000, 16, Permissions::READ_WRITE)?,
    )?;
    let mut queue = SubmissionQueue::new(&gpu, QueueLimits::default())?;
    let mut entries = wire(address, 2);
    entries.extend(wire(address + 8, 2));
    let first = queue.submit(&gpu, &entries)?;
    require(
        queue
            .front()
            .ok_or("No captured batch")?
            .writes()
            .iter()
            .map(|w| w.data)
            .collect::<Vec<_>>()
            == [10, 20, 30],
        "A packet split across entries decoded incorrectly",
    )?;
    cpu.write_u32(0x1004, 99)?;
    require(
        queue.front().ok_or("Lost first batch")?.writes()[0].data == 10,
        "CPU mutation changed captured commands",
    )?;
    cpu.unmap(0x1000, 16)?;
    gpu.unmap(id)?;
    require(
        cpu.resident_bytes() == 16,
        "Queued backing was released before its batch",
    )?;
    cpu.map(
        0x1000,
        0x807b_0044_u32.to_le_bytes().to_vec(),
        Permissions::READ,
    )?;
    let replacement = gpu.map(GpuVa(address), cpu.pin(0x1000, 4, Permissions::READ)?)?;
    let second = queue.submit(&gpu, &wire(address, 1))?;
    let admitted = queue.stats();
    require(
        admitted.pending == 2 && admitted.decoded_writes == 4,
        "Queue accounting mismatch",
    )?;
    require(
        queue.release_front(second).is_err() && queue.stats() == admitted,
        "Out-of-order release mutated the queue",
    )?;
    require(
        cpu.resident_bytes() == 20,
        "Address reuse lost the original backing charge",
    )?;
    queue.release_front(first)?;
    require(
        cpu.resident_bytes() == 4
            && queue.front().ok_or("Lost second batch")?.writes()[0].data == 123,
        "Releasing the original batch damaged the replacement",
    )?;
    cpu.unmap(0x1000, 4)?;
    gpu.unmap(replacement)?;
    require(
        cpu.resident_bytes() == 4,
        "Pending replacement was reclaimed too early",
    )?;
    queue.release_front(second)?;
    require(
        cpu.resident_bytes() == 0 && queue.stats() == QueueStats::default(),
        "Final release leaked queue/backing resources",
    )?;
    Ok((
        admitted.pending,
        admitted.decoded_writes,
        cpu.resident_bytes(),
        0,
        0,
    ))
}
fn dma_copy() -> Result<Metrics, Error> {
    let mut cpu = AddressSpace::with_limit(1024);
    let command = 0x1_0000_1000;
    let data = 0x2_0000_0000;
    let alias = 0x4_0000_0000;
    // Bind class, initialize its five required registers, launch, then copy
    // the just-written bytes through a second virtual alias of that backing.
    let methods = [
        (0, MAXWELL_DMA_COPY_A),
        (0x400, 2),
        (0x404, 0),
        (0x408, 2),
        (0x40c, 16),
        (0x418, 8),
        (0x300, LINEAR_DMA_LAUNCH),
        (0x400, 4),
        (0x404, 16),
        (0x40c, 32),
        (0x300, LINEAR_DMA_LAUNCH),
    ];
    let bytes: Vec<u8> = methods
        .into_iter()
        .flat_map(|(offset, value)| [0x2001_0000_u32 | (offset / 4), value])
        .flat_map(u32::to_le_bytes)
        .collect();
    let size = bytes.len();
    cpu.map(0x1000, bytes, Permissions::READ_WRITE)?;
    cpu.map(0x3000, (0..64).collect(), Permissions::READ_WRITE)?;
    let mut gpu = GpuAddressSpace::new(1024)?;
    let command_map = gpu.map(GpuVa(command), cpu.pin(0x1000, size, Permissions::READ)?)?;
    let data_map = gpu.map(GpuVa(data), cpu.pin(0x3000, 64, Permissions::READ_WRITE)?)?;
    let alias_map = gpu.map(GpuVa(alias), cpu.pin(0x3000, 64, Permissions::READ_WRITE)?)?;
    let mut queue = SubmissionQueue::new(&gpu, QueueLimits::default())?;
    let id = queue.submit(&gpu, &wire(command, (size / 4) as u32))?;
    let admitted = queue.stats();
    // Snapshot commands are unaffected; live source data changes before execution.
    cpu.write(0x1000, &vec![0xff; size])?;
    let expected = [0xa5, 0x17, 0x39, 0x51, 0x63, 0x8b, 0xcd, 0xef];
    cpu.write(0x3000, &expected)?;
    let stats = queue.execute_front(&gpu, id, ExecutionLimits::default())?;
    let mut actual = [0; 8];
    cpu.read(0x3010, &mut actual)?;
    require(actual == expected, "First DMA launch copied wrong bytes")?;
    cpu.read(0x3020, &mut actual)?;
    require(
        actual == expected,
        "Dependent DMA launch missed aliased writes",
    )?;
    require(
        stats.methods == 11 && stats.copy_launches == 2 && stats.copied_bytes == 16,
        "DMA execution counters mismatch",
    )?;
    require(
        queue
            .execute_front(&gpu, id, ExecutionLimits::default())
            .is_err(),
        "A consumed DMA batch executed twice",
    )?;
    gpu.unmap(command_map)?;
    gpu.unmap(data_map)?;
    gpu.unmap(alias_map)?;
    cpu.unmap(0x1000, size)?;
    cpu.unmap(0x3000, 64)?;
    require(
        cpu.resident_bytes() == 0 && queue.stats() == QueueStats::default(),
        "Completed DMA retained backing or pending work",
    )?;
    Ok((
        admitted.pending,
        admitted.decoded_writes,
        cpu.resident_bytes(),
        stats.copy_launches,
        stats.copied_bytes,
    ))
}
pub fn report() -> String {
    let mut all = true;
    let mut tests = Vec::new();
    for (name, run) in [
        ("shared-cpu-gpu-backing-aliases", aliases as Check),
        ("queued-command-snapshot-and-lifetime", submission as Check),
        ("maxwell-dma-linear-copy", dma_copy as Check),
    ] {
        let result = run();
        let passed = result.is_ok();
        all &= passed;
        let (captures, writes, resident, launches, copied, error) = match result {
            Ok((a, b, c, d, e)) => (a, b, c, d, e, String::new()),
            Err(e) => (0, 0, 0, 0, 0, e.to_string()),
        };
        tests.push(format!("{{\"name\":{},\"passed\":{},\"capturedSubmissions\":{},\"decodedWrites\":{},\"residentBytesAfterRelease\":{},\"copyLaunches\":{},\"copiedBytes\":{},\"error\":{}}}",super::quote(name),passed,captures,writes,resident,launches,copied,super::quote(&error)));
    }
    for case in super::textures::cases()
        .into_iter()
        .chain(super::indexed::cases())
    {
        all &= case.passed;
        tests.push(format!(
            "{{\"name\":{},\"passed\":{},\"verifiedItems\":{},\"error\":{}}}",
            super::quote(case.name),
            case.passed,
            case.checks,
            super::quote(&case.error)
        ));
    }
    format!("{{\"scope\":\"original-gpu-memory-and-command-reference\",\"suiteVersion\":4,\"hostArchitecture\":{},\"tests\":[{}],\"allPassed\":{}}}",super::quote(std::env::consts::ARCH),tests.join(","),all)
}
