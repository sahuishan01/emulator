//! Actual CPU/GPU backing and command-capture checks, separate from guest NROs
//! and from native Vulkan. No GPU methods, shaders or fences execute here.
use emulator_gpu::{GpuAddressSpace, GpuVa, QueueLimits, QueueStats, SubmissionQueue};
use emulator_memory::{AddressSpace, Permissions};
type Error = Box<dyn std::error::Error>;
type Check = fn() -> Result<(usize, usize, usize), Error>;
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

fn aliases() -> Result<(usize, usize, usize), Error> {
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
    Ok((0, 0, cpu.resident_bytes()))
}
fn submission() -> Result<(usize, usize, usize), Error> {
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
    ))
}
pub fn report() -> String {
    let mut all = true;
    let mut tests = Vec::new();
    for (name, run) in [
        ("shared-cpu-gpu-backing-aliases", aliases as Check),
        ("queued-command-snapshot-and-lifetime", submission as Check),
    ] {
        let result = run();
        let passed = result.is_ok();
        all &= passed;
        let (captures, writes, resident, error) = match result {
            Ok((a, b, c)) => (a, b, c, String::new()),
            Err(e) => (0, 0, 0, e.to_string()),
        };
        tests.push(format!("{{\"name\":{},\"passed\":{},\"capturedSubmissions\":{},\"decodedWrites\":{},\"residentBytesAfterRelease\":{},\"error\":{}}}",super::quote(name),passed,captures,writes,resident,super::quote(&error)));
    }
    format!("{{\"scope\":\"original-gpu-memory-and-command-reference\",\"suiteVersion\":1,\"hostArchitecture\":{},\"tests\":[{}],\"allPassed\":{}}}",super::quote(std::env::consts::ARCH),tests.join(","),all)
}
