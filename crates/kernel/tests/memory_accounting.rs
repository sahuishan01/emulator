//! Physical memory accounting: GetInfo 6/7/16/17/21/22 and resource-limit
//! enforcement for svcMapPhysicalMemory and svcSetHeapSize.

use emulator_a64::Cpu;
use emulator_kernel::{
    Kernel, ALIAS_REGION_BASE, APPLICATION_POOL_SIZE, RESULT_LIMIT_REACHED, SVC_GET_INFO,
    SVC_MAP_PHYSICAL_MEMORY, SVC_SET_HEAP_SIZE, SVC_UNMAP_PHYSICAL_MEMORY,
};
use emulator_memory::{AddressSpace, Permissions};

const SYSRES: u64 = 0x100_0000;
const IMAGE: u64 = 0x10_0000;

fn info(kernel: &mut Kernel, memory: &mut AddressSpace, kind: u64) -> u64 {
    let mut cpu = Cpu::new(0, 0);
    (cpu.x[0], cpu.x[1], cpu.x[2], cpu.x[3]) = (0, kind, 0xffff_8001, 0);
    kernel.dispatch(SVC_GET_INFO, &mut cpu, memory).unwrap();
    assert_eq!(cpu.x[0], 0, "GetInfo {kind}");
    cpu.x[1]
}

fn call(kernel: &mut Kernel, memory: &mut AddressSpace, id: u16, a: u64, b: u64) -> u64 {
    let mut cpu = Cpu::new(0, 0);
    (cpu.x[0], cpu.x[1]) = (a, b);
    kernel.dispatch(id, &mut cpu, memory).unwrap();
    cpu.x[0]
}

fn process() -> (Kernel, AddressSpace) {
    let mut kernel = Kernel::default();
    kernel.set_system_resource_size(SYSRES);
    kernel.set_fixed_memory_charge(IMAGE);
    (kernel, AddressSpace::with_limit(0x1000_0000))
}

#[test]
fn non_system_totals_exclude_the_system_resource_and_count_only_charged_pages() {
    let (mut kernel, mut memory) = process();
    assert_eq!(info(&mut kernel, &mut memory, 6), APPLICATION_POOL_SIZE);
    assert_eq!(
        info(&mut kernel, &mut memory, 21),
        APPLICATION_POOL_SIZE - SYSRES
    );
    assert_eq!(info(&mut kernel, &mut memory, 22), IMAGE);
    assert_eq!(info(&mut kernel, &mut memory, 7), IMAGE + SYSRES);
    assert_eq!(info(&mut kernel, &mut memory, 16), SYSRES);
    assert_eq!(info(&mut kernel, &mut memory, 17), 0);

    // Pages a service maps into the process (shared memory) are not charged.
    memory
        .map(0x7000_0000, vec![0; 0x4_0000], Permissions::READ)
        .unwrap();
    assert_eq!(info(&mut kernel, &mut memory, 22), IMAGE);

    // Physical mappings are charged; the system resource holds their tables.
    let mapped = 0x40_0000;
    assert_eq!(
        call(
            &mut kernel,
            &mut memory,
            SVC_MAP_PHYSICAL_MEMORY,
            ALIAS_REGION_BASE,
            mapped
        ),
        0
    );
    assert_eq!(info(&mut kernel, &mut memory, 22), IMAGE + mapped);
    assert_eq!(info(&mut kernel, &mut memory, 7), IMAGE + mapped + SYSRES);
    assert_eq!(info(&mut kernel, &mut memory, 17), 0x2000);

    // Heap growth is charged too, and unmapping releases the charge.
    let mut cpu = Cpu::new(0, 0);
    cpu.x[1] = 0x20_0000;
    kernel
        .dispatch(SVC_SET_HEAP_SIZE, &mut cpu, &mut memory)
        .unwrap();
    assert_eq!(cpu.x[0], 0);
    assert_eq!(
        info(&mut kernel, &mut memory, 22),
        IMAGE + mapped + 0x20_0000
    );
    assert_eq!(
        call(
            &mut kernel,
            &mut memory,
            SVC_UNMAP_PHYSICAL_MEMORY,
            ALIAS_REGION_BASE,
            mapped
        ),
        0
    );
    assert_eq!(info(&mut kernel, &mut memory, 22), IMAGE + 0x20_0000);
    assert_eq!(info(&mut kernel, &mut memory, 17), 0);
}

#[test]
fn growth_past_the_limit_fails_with_limit_reached_and_maps_nothing() {
    let (mut kernel, mut memory) = process();
    let total = APPLICATION_POOL_SIZE - SYSRES;
    // Leave exactly one page of headroom under the limit.
    kernel.set_fixed_memory_charge(total - 0x1000);
    let resident = memory.resident_bytes();

    assert_eq!(
        call(
            &mut kernel,
            &mut memory,
            SVC_MAP_PHYSICAL_MEMORY,
            ALIAS_REGION_BASE,
            0x2000
        ),
        RESULT_LIMIT_REACHED
    );
    assert_eq!(memory.resident_bytes(), resident);
    assert!(memory.query(ALIAS_REGION_BASE).is_none());
    assert_eq!(kernel.physical_mapped(), 0);

    let mut cpu = Cpu::new(0, 0);
    cpu.x[1] = 0x2000;
    kernel
        .dispatch(SVC_SET_HEAP_SIZE, &mut cpu, &mut memory)
        .unwrap();
    assert_eq!(cpu.x[0], RESULT_LIMIT_REACHED);
    assert_eq!(memory.resident_bytes(), resident);

    // The last page still fits, after which used == total and 21 - 22 == 0.
    assert_eq!(
        call(
            &mut kernel,
            &mut memory,
            SVC_MAP_PHYSICAL_MEMORY,
            ALIAS_REGION_BASE,
            0x1000
        ),
        0
    );
    assert_eq!(
        info(&mut kernel, &mut memory, 22),
        info(&mut kernel, &mut memory, 21)
    );
    assert_eq!(
        call(
            &mut kernel,
            &mut memory,
            SVC_MAP_PHYSICAL_MEMORY,
            ALIAS_REGION_BASE + 0x1000,
            0x1000
        ),
        RESULT_LIMIT_REACHED
    );
}
