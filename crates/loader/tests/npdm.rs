use emulator_loader::npdm::{NpdmMeta, NPDM_HEADER_SIZE, NPDM_MAGIC};

#[test]
fn test_npdm_parser() {
    let mut header = vec![0u8; NPDM_HEADER_SIZE];
    header[0..4].copy_from_slice(NPDM_MAGIC);

    // Product code: "PROD-12345"
    let prod = b"PROD-12345";
    header[0x10..0x10 + prod.len()].copy_from_slice(prod);

    // Title ID: 0x0100000000010000
    header[0x20..0x28].copy_from_slice(&0x0100_0000_0001_0000u64.to_le_bytes());

    // Priority: 44
    header[0x28..0x2c].copy_from_slice(&44i32.to_le_bytes());

    // Default core: 0
    header[0x2c..0x30].copy_from_slice(&0i32.to_le_bytes());

    // Stack size: 1 MiB (0x100000)
    header[0x34..0x38].copy_from_slice(&0x10_0000u32.to_le_bytes());

    let meta = NpdmMeta::parse(&header).expect("valid NPDM header");
    assert_eq!(meta.product_code, "PROD-12345");
    assert_eq!(meta.title_id, 0x0100_0000_0001_0000);
    assert_eq!(meta.main_thread_priority, 44);
    assert_eq!(meta.default_cpu_core, 0);
    assert_eq!(meta.main_thread_stack_size, 0x10_0000);
}
