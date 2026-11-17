use emulator_a64::Cpu;
use emulator_kernel::{
    service::ServiceError, Kernel, KernelError, SVC_CLOSE_HANDLE, SVC_CONNECT_TO_NAMED_PORT,
    SVC_CREATE_TRANSFER_MEMORY, SVC_SEND_SYNC_REQUEST,
};
use emulator_memory::{AddressSpace, Permissions};
use emulator_services::hwopus::{OpusDecoderManager, OpusParameters, RESULT_OPUS_INVALID_ARGUMENT};

const TMEM_SIZE: u64 = 0x2_0000;

struct Rig {
    k: Kernel,
    c: Cpu,
    m: AddressSpace,
    tmem: u32,
    session: u32,
}

impl Rig {
    fn new() -> Self {
        let mut k = Kernel::default();
        let mut c = Cpu::new(0, 0);
        let mut m = AddressSpace::default();
        m.map(0x1000, b"hwopus\0".to_vec(), Permissions::READ)
            .unwrap();
        m.map(0x2000, vec![0; 256], Permissions::READ_WRITE)
            .unwrap();
        m.map(
            0x10000,
            vec![0; TMEM_SIZE as usize],
            Permissions::READ_WRITE,
        )
        .unwrap();
        c.x[1] = 0x10000;
        c.x[2] = TMEM_SIZE;
        c.x[3] = 0;
        k.dispatch(SVC_CREATE_TRANSFER_MEMORY, &mut c, &mut m)
            .unwrap();
        let tmem = c.x[1] as u32;
        k.register_named_port("hwopus", Box::new(OpusDecoderManager))
            .unwrap();
        c.tpidrro_el0 = 0x2000;
        c.x[1] = 0x1000;
        k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m)
            .unwrap();
        let session = c.x[1] as u32;
        Self {
            k,
            c,
            m,
            tmem,
            session,
        }
    }

    /// CMIF request with an optional single copied handle and raw words.
    fn request(
        &mut self,
        command: u32,
        handle: Option<u32>,
        words: &[u32],
    ) -> Result<(), KernelError> {
        let mut b = [0u8; 256];
        let raw_words = 4 + 4 + words.len() as u32; // padding + header + payload
        b[..4].copy_from_slice(&4_u32.to_le_bytes());
        let mut at = 8;
        if let Some(h) = handle {
            b[4..8].copy_from_slice(&(0x8000_0000 | raw_words).to_le_bytes());
            b[8..12].copy_from_slice(&2_u32.to_le_bytes());
            b[12..16].copy_from_slice(&h.to_le_bytes());
            at = 16;
        } else {
            b[4..8].copy_from_slice(&raw_words.to_le_bytes());
        }
        at = (at + 15) & !15;
        b[at..at + 4].copy_from_slice(b"SFCI");
        b[at + 8..at + 12].copy_from_slice(&command.to_le_bytes());
        for (i, w) in words.iter().enumerate() {
            let o = at + 16 + i * 4;
            b[o..o + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.m.write(0x2000, &b).unwrap();
        self.c.x[0] = self.session as u64;
        self.k
            .dispatch(SVC_SEND_SYNC_REQUEST, &mut self.c, &mut self.m)
            .map(|_| ())
    }

    /// Result word and first output word of a reply without handles.
    fn plain_reply(&self) -> (u32, u32) {
        (
            self.m.read_u32(0x2018).unwrap(),
            self.m.read_u32(0x2020).unwrap(),
        )
    }
}

#[test]
fn work_buffer_size_is_validated_and_page_aligned() {
    let mut r = Rig::new();
    r.request(1, None, &[48_000, 2]).unwrap();
    let (result, size) = r.plain_reply();
    assert_eq!(result, 0);
    assert_eq!(size % 0x1000, 0);
    let expected = OpusParameters {
        sample_rate: 48_000,
        channel_count: 2,
        large_frames: false,
    };
    assert_eq!(size, expected.work_buffer_size());
    // Ex with large frames needs more.
    r.request(5, None, &[48_000, 2, 1, 0]).unwrap();
    let large = r.plain_reply().1;
    assert!(large > size);
    r.request(8, None, &[48_000, 2, 1, 0]).unwrap();
    assert_eq!(r.plain_reply(), (0, large));
    for bad in [[44_100, 2], [48_000, 3], [48_000, 0]] {
        r.request(1, None, &bad).unwrap();
        assert_eq!(r.plain_reply().0, RESULT_OPUS_INVALID_ARGUMENT);
    }
}

#[test]
fn open_retains_transfer_memory_and_decode_fails_explicitly() {
    let mut r = Rig::new();
    let size = OpusParameters {
        sample_rate: 48_000,
        channel_count: 2,
        large_frames: false,
    }
    .work_buffer_size();
    assert!(u64::from(size) <= TMEM_SIZE);
    r.request(0, Some(r.tmem), &[48_000, 2, size]).unwrap();
    // Reply carries a moved handle: descriptor present.
    assert_ne!(r.m.read_u32(0x2004).unwrap() & 0x8000_0000, 0);
    let decoder = r.m.read_u32(0x200c).unwrap();
    r.c.x[0] = r.tmem as u64;
    r.k.dispatch(SVC_CLOSE_HANDLE, &mut r.c, &mut r.m).unwrap();
    assert_eq!(
        r.k.live_transfer_memory_count(),
        1,
        "decoder keeps work buffer"
    );
    r.session = decoder;
    assert!(matches!(
        r.request(0, None, &[]),
        Err(KernelError::Service(ServiceError::UnsupportedCommand {
            service: "IHardwareOpusDecoder",
            ..
        }))
    ));
    r.c.x[0] = decoder as u64;
    r.k.dispatch(SVC_CLOSE_HANDLE, &mut r.c, &mut r.m).unwrap();
    assert_eq!(r.k.live_transfer_memory_count(), 0);
}

#[test]
fn open_rejects_missing_handle_and_undersized_buffers() {
    let mut r = Rig::new();
    assert_eq!(
        r.request(0, None, &[48_000, 2, 0x1_0000]),
        Err(KernelError::Service(ServiceError::InvalidTransferMemory))
    );
    r.request(0, Some(r.tmem), &[48_000, 2, 0x1000]).unwrap();
    assert_eq!(r.plain_reply().0, RESULT_OPUS_INVALID_ARGUMENT);
    r.request(0, Some(r.tmem), &[48_000, 2, TMEM_SIZE as u32 * 2])
        .unwrap();
    assert_eq!(r.plain_reply().0, RESULT_OPUS_INVALID_ARGUMENT);
    assert!(matches!(
        r.request(2, None, &[]),
        Err(KernelError::Service(ServiceError::UnsupportedCommand {
            service: "hwopus",
            ..
        }))
    ));
}
