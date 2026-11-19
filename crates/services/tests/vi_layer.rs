//! IApplicationDisplayService layer commands driven through svcSendSyncRequest
//! with a real B (receive) buffer for the native-window parcel.

use emulator_a64::Cpu;
use emulator_kernel::{Kernel, SVC_CONNECT_TO_NAMED_PORT, SVC_SEND_SYNC_REQUEST};
use emulator_memory::{AddressSpace, Permissions};
use emulator_services::vi::{native_window_parcel, DisplayService, LAYER_BINDER_ID};

const MESSAGE: u64 = 0x2000;
const OUTPUT: u64 = 0x4000;

struct Rig {
    kernel: Kernel,
    cpu: Cpu,
    memory: AddressSpace,
    session: u32,
}

impl Rig {
    fn new() -> Self {
        let mut kernel = Kernel::default();
        let mut cpu = Cpu::new(0, 0);
        let mut memory = AddressSpace::default();
        memory
            .map(0x1000, b"vi:t\0".to_vec(), Permissions::READ)
            .unwrap();
        memory
            .map(MESSAGE, vec![0; 256], Permissions::READ_WRITE)
            .unwrap();
        memory
            .map(OUTPUT, vec![0xee; 4096], Permissions::READ_WRITE)
            .unwrap();
        kernel
            .register_named_port("vi:t", Box::new(DisplayService::default()))
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
        }
    }

    /// CMIF request with an optional B buffer; returns (result, raw reply data).
    fn call(
        &mut self,
        command: u32,
        payload: &[u8],
        buffer: Option<(u64, u32)>,
    ) -> Option<(u32, Vec<u8>)> {
        let mut b = [0u8; 256];
        let b_count = u32::from(buffer.is_some());
        let words = (16 + payload.len() as u32 + 3) / 4 + 4;
        b[..4].copy_from_slice(&(4 | (b_count << 24)).to_le_bytes());
        b[4..8].copy_from_slice(&words.to_le_bytes());
        if let Some((address, size)) = buffer {
            b[8..12].copy_from_slice(&size.to_le_bytes());
            b[12..16].copy_from_slice(&(address as u32).to_le_bytes());
        }
        let raw = if buffer.is_some() { 32 } else { 16 };
        b[raw..raw + 4].copy_from_slice(b"SFCI");
        b[raw + 8..raw + 12].copy_from_slice(&command.to_le_bytes());
        b[raw + 16..raw + 16 + payload.len()].copy_from_slice(payload);
        self.memory.write(MESSAGE, &b).unwrap();
        self.cpu.x[0] = u64::from(self.session);
        self.kernel
            .dispatch(SVC_SEND_SYNC_REQUEST, &mut self.cpu, &mut self.memory)
            .map_err(|error| eprintln!("command {command}: {error:?}"))
            .ok()?;
        let result = self.kernel.ipc_trace().last()?.result?;
        let mut reply = [0u8; 64];
        self.memory.read(MESSAGE, &mut reply).unwrap();
        // Reply raw data: header(8) + pad to 16 + "SFCO" header(16).
        Some((result, reply[32..64].to_vec()))
    }

    fn output(&self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        self.memory.read(OUTPUT, &mut bytes).unwrap();
        bytes
    }
}

fn open_layer_payload(name: &[u8], layer: u64) -> Vec<u8> {
    let mut payload = vec![0u8; 0x50];
    payload[..name.len()].copy_from_slice(name);
    payload[0x40..0x48].copy_from_slice(&layer.to_le_bytes());
    payload[0x48..0x50].copy_from_slice(&0x55u64.to_le_bytes()); // ARUID
    payload
}

#[test]
fn open_layer_writes_native_window_parcel_into_the_receive_buffer() {
    let parcel = native_window_parcel(LAYER_BINDER_ID);
    assert_eq!(parcel.len(), 0x3c);
    assert_eq!(&parcel[0x10 + 0x18..0x10 + 0x1f], b"dispdrv");

    let mut rig = Rig::new();
    let (result, data) = rig
        .call(
            2020,
            &open_layer_payload(b"Default", 1),
            Some((OUTPUT, 0x210)),
        )
        .unwrap();
    assert_eq!(result, 0);
    assert_eq!(
        u64::from_le_bytes(data[..8].try_into().unwrap()),
        parcel.len() as u64
    );
    assert_eq!(rig.output(parcel.len()), parcel);
    assert_eq!(
        rig.output(parcel.len() + 8)[parcel.len()..],
        [0xee; 8],
        "only the parcel is written"
    );

    // The recording layer of a separable layer pair opens with its own binder.
    let (result, _) = rig
        .call(
            2020,
            &open_layer_payload(b"Default", 2),
            Some((OUTPUT, 0x210)),
        )
        .unwrap();
    assert_eq!(result, 0);
    assert_eq!(rig.output(0x3c), native_window_parcel(2));

    // CreateStrayLayer(flags, display 0) -> layer 1 + parcel size.
    let mut stray = [0u8; 16];
    stray[..4].copy_from_slice(&1u32.to_le_bytes());
    let (result, data) = rig.call(2030, &stray, Some((OUTPUT, 0x210))).unwrap();
    assert_eq!(result, 0);
    assert_eq!(u64::from_le_bytes(data[..8].try_into().unwrap()), 1);
    assert_eq!(
        u64::from_le_bytes(data[8..16].try_into().unwrap()),
        parcel.len() as u64
    );
}

#[test]
fn open_layer_rejects_unknown_display_layer_small_buffer_and_missing_buffer() {
    let mut rig = Rig::new();
    let before = rig.output(0x40);
    assert!(rig
        .call(
            2020,
            &open_layer_payload(b"External", 1),
            Some((OUTPUT, 0x210))
        )
        .is_none());
    assert!(rig
        .call(
            2020,
            &open_layer_payload(b"Default", 7),
            Some((OUTPUT, 0x210))
        )
        .is_none());
    assert!(rig
        .call(
            2020,
            &open_layer_payload(b"Default", 1),
            Some((OUTPUT, 0x10))
        )
        .is_none());
    assert!(rig
        .call(2020, &open_layer_payload(b"Default", 1), None)
        .is_none());
    assert_eq!(
        rig.output(0x40),
        before,
        "rejected requests leave guest memory untouched"
    );
}
