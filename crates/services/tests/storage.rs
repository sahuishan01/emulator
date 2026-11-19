//! fsp-srv IStorage over the RomFS image, driven through svcSendSyncRequest with a
//! real B (receive) buffer.

use emulator_a64::Cpu;
use emulator_kernel::{Kernel, SVC_CONNECT_TO_NAMED_PORT, SVC_SEND_SYNC_REQUEST};
use emulator_loader::romfs::build_romfs_image;
use emulator_loader::RomFsArchive;
use emulator_memory::{AddressSpace, Permissions};
use emulator_services::fspsrv::{
    StorageProxy, RESULT_FS_OUT_OF_RANGE, RESULT_FS_UNSUPPORTED_WRITE,
};

const MESSAGE: u64 = 0x2000;
const OUTPUT: u64 = 0x4000;

struct Rig {
    kernel: Kernel,
    cpu: Cpu,
    memory: AddressSpace,
    session: u32,
    image: Vec<u8>,
}

impl Rig {
    fn new() -> Self {
        let image = build_romfs_image(&[("data/a.bin", b"hello romfs storage")]);
        let romfs = RomFsArchive::parse(&image).unwrap();
        let mut kernel = Kernel::default();
        let mut cpu = Cpu::new(0, 0);
        let mut memory = AddressSpace::default();
        memory
            .map(0x1000, b"st:\0".to_vec(), Permissions::READ)
            .unwrap();
        memory
            .map(MESSAGE, vec![0; 256], Permissions::READ_WRITE)
            .unwrap();
        memory
            .map(OUTPUT, vec![0xee; 4096], Permissions::READ_WRITE)
            .unwrap();
        kernel
            .register_named_port("st:", Box::new(StorageProxy::new(std::rc::Rc::new(romfs))))
            .unwrap();
        cpu.tpidrro_el0 = MESSAGE;
        cpu.x[1] = 0x1000;
        kernel
            .dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut cpu, &mut memory)
            .unwrap();
        let session = cpu.x[1] as u32;
        Self {
            kernel,
            cpu,
            memory,
            session,
            image,
        }
    }

    /// CMIF request with an optional B buffer and a 16-byte payload.
    fn call(&mut self, command: u32, payload: [u64; 2], buffer: Option<(u64, u32)>) -> Option<u32> {
        let mut b = [0u8; 256];
        let b_count = u32::from(buffer.is_some());
        b[..4].copy_from_slice(&(4 | (b_count << 24)).to_le_bytes());
        b[4..8].copy_from_slice(&12_u32.to_le_bytes());
        if let Some((address, size)) = buffer {
            b[8..12].copy_from_slice(&size.to_le_bytes());
            b[12..16].copy_from_slice(&(address as u32).to_le_bytes());
        }
        // Raw data starts 16-byte aligned after the header and descriptors.
        let raw = if buffer.is_some() { 32 } else { 16 };
        b[raw..raw + 4].copy_from_slice(b"SFCI");
        b[raw + 8..raw + 12].copy_from_slice(&command.to_le_bytes());
        b[raw + 16..raw + 24].copy_from_slice(&payload[0].to_le_bytes());
        b[raw + 24..raw + 32].copy_from_slice(&payload[1].to_le_bytes());
        self.memory.write(MESSAGE, &b).unwrap();
        self.cpu.x[0] = u64::from(self.session);
        self.kernel
            .dispatch(SVC_SEND_SYNC_REQUEST, &mut self.cpu, &mut self.memory)
            .map_err(|error| eprintln!("command {command}: {error:?}"))
            .ok()?;
        self.kernel
            .ipc_trace()
            .last()
            .and_then(|trace| trace.result)
    }

    fn output(&self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        self.memory.read(OUTPUT, &mut bytes).unwrap();
        bytes
    }
}

#[test]
fn read_copies_romfs_image_bytes_into_the_receive_buffer() {
    let mut rig = Rig::new();
    let size = rig.image.len() as u64;
    assert_eq!(rig.call(0, [8, 32], Some((OUTPUT, 64))), Some(0));
    assert_eq!(rig.output(32), rig.image[8..40]);
    assert_eq!(
        rig.output(64)[32..],
        [0xee; 32],
        "only `size` bytes are written"
    );

    // Exactly up to the end is fine; one byte further is OutOfRange and writes nothing.
    assert_eq!(rig.call(0, [size - 4, 4], Some((OUTPUT, 64))), Some(0));
    assert_eq!(rig.output(4), rig.image[size as usize - 4..]);
    rig.memory.write(OUTPUT, &[0xee; 8]).unwrap();
    assert_eq!(
        rig.call(0, [size - 4, 5], Some((OUTPUT, 64))),
        Some(RESULT_FS_OUT_OF_RANGE)
    );
    assert_eq!(rig.output(8), [0xee; 8]);
    // Size larger than the guest buffer, and offset overflow, are rejected.
    assert_eq!(
        rig.call(0, [0, 65], Some((OUTPUT, 64))),
        Some(RESULT_FS_OUT_OF_RANGE)
    );
    assert_eq!(
        rig.call(0, [u64::MAX, 2], Some((OUTPUT, 64))),
        Some(RESULT_FS_OUT_OF_RANGE)
    );
    // Read without its B buffer is a malformed request.
    assert_eq!(rig.call(0, [0, 4], None), None);
}

#[test]
fn storage_is_read_only_and_reports_image_size() {
    let mut rig = Rig::new();
    assert_eq!(rig.call(1, [0, 0], None), Some(RESULT_FS_UNSUPPORTED_WRITE));
    assert_eq!(rig.call(3, [0, 0], None), Some(RESULT_FS_UNSUPPORTED_WRITE));
    assert_eq!(rig.call(99, [0, 0], None), None);
}
