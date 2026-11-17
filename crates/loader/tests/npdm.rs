use emulator_loader::npdm::{NpdmMeta, ACI0_MAGIC, NPDM_HEADER_SIZE, NPDM_MAGIC};

fn header() -> Vec<u8> {
    let mut header = vec![0u8; NPDM_HEADER_SIZE];
    header[0..4].copy_from_slice(NPDM_MAGIC);
    header
}

#[test]
fn test_npdm_parser() {
    let mut npdm = header();
    npdm[0x0c] = 0b0111; // 64-bit, address space type 3 (39-bit)
    npdm[0x0e] = 44;
    npdm[0x0f] = 0;
    npdm[0x14..0x18].copy_from_slice(&0x0180_0000u32.to_le_bytes());
    npdm[0x18..0x1c].copy_from_slice(&3u32.to_le_bytes());
    npdm[0x1c..0x20].copy_from_slice(&0x10_0000u32.to_le_bytes());
    npdm[0x20..0x27].copy_from_slice(b"Program");
    npdm[0x30..0x3a].copy_from_slice(b"PROD-12345");
    // ACI0 right after the header.
    let mut aci = vec![0u8; 0x40];
    aci[0..4].copy_from_slice(ACI0_MAGIC);
    aci[0x10..0x18].copy_from_slice(&0x0100_0000_0001_0000u64.to_le_bytes());
    npdm[0x70..0x74].copy_from_slice(&(NPDM_HEADER_SIZE as u32).to_le_bytes());
    npdm[0x74..0x78].copy_from_slice(&(aci.len() as u32).to_le_bytes());
    npdm.extend_from_slice(&aci);

    let meta = NpdmMeta::parse(&npdm).expect("valid NPDM header");
    assert_eq!(meta.product_code, "PROD-12345");
    assert_eq!(meta.name, "Program");
    assert_eq!(meta.title_id, 0x0100_0000_0001_0000);
    assert_eq!(meta.main_thread_priority, 44);
    assert_eq!(meta.default_cpu_core, 0);
    assert_eq!(meta.main_thread_stack_size, 0x10_0000);
    assert_eq!(meta.system_resource_size, 0x0180_0000);
    assert_eq!(meta.version, 3);
    assert!(meta.is_64bit);
    assert_eq!(meta.address_space_type, 3);
}

#[test]
fn test_npdm_without_aci_has_no_program_id() {
    let meta = NpdmMeta::parse(&header()).unwrap();
    assert_eq!(meta.title_id, 0);
    assert_eq!(meta.system_resource_size, 0);
}

#[test]
fn test_npdm_rejects_bad_aci() {
    let mut npdm = header();
    npdm[0x70..0x74].copy_from_slice(&0x80u32.to_le_bytes());
    npdm[0x74..0x78].copy_from_slice(&0x40u32.to_le_bytes());
    assert!(NpdmMeta::parse(&npdm).is_err()); // out of bounds
    npdm.extend_from_slice(&[0u8; 0x40]);
    assert!(NpdmMeta::parse(&npdm).is_err()); // wrong magic
    assert!(NpdmMeta::parse(&npdm[..0x40]).is_err()); // short header
}
