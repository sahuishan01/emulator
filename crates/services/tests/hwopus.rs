use emulator_a64::Cpu;
use emulator_kernel::{
    service::ServiceError, Kernel, KernelError, SVC_CLOSE_HANDLE, SVC_CONNECT_TO_NAMED_PORT,
    SVC_CREATE_TRANSFER_MEMORY, SVC_SEND_SYNC_REQUEST,
};
use emulator_memory::{AddressSpace, Permissions};
use emulator_opus::Decoder;
use emulator_services::hwopus::{
    OpusDecoderManager, OpusParameters, RESULT_OPUS_BUFFER_TOO_SMALL, RESULT_OPUS_INPUT_TOO_SMALL,
    RESULT_OPUS_INVALID_ARGUMENT, RESULT_OPUS_INVALID_PACKET,
};

const TMEM_SIZE: u64 = 0x2_0000;
const INPUT: u64 = 0x40000;
const OUTPUT: u64 = 0x50000;
const OUTPUT_SIZE: u64 = 0x10000;

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
        m.map(INPUT, vec![0; 0x1000], Permissions::READ_WRITE).unwrap();
        m.map(OUTPUT, vec![0; OUTPUT_SIZE as usize], Permissions::READ_WRITE)
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

    /// CMIF request on the current session with A (send) and B (receive)
    /// map-alias buffers and raw payload bytes.
    fn buffers(&mut self, command: u32, a: &[(u64, u64)], b: &[(u64, u64)], data: &[u8]) -> Result<(), KernelError> {
        let mut p = [0u8; 256];
        let word0 = 4 | ((a.len() as u32) << 20) | ((b.len() as u32) << 24);
        let raw_words = 8 + data.len().div_ceil(4) as u32;
        p[..4].copy_from_slice(&word0.to_le_bytes());
        p[4..8].copy_from_slice(&raw_words.to_le_bytes());
        let mut at = 8;
        for &(address, size) in a.iter().chain(b) {
            for (i, v) in [size as u32, address as u32, 0].into_iter().enumerate() {
                p[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            at += 12;
        }
        let header = (at + 15) & !15;
        p[header..header + 4].copy_from_slice(b"SFCI");
        p[header + 8..header + 12].copy_from_slice(&command.to_le_bytes());
        p[header + 16..header + 16 + data.len()].copy_from_slice(data);
        self.m.write(0x2000, &p).unwrap();
        self.c.x[0] = self.session as u64;
        self.k
            .dispatch(SVC_SEND_SYNC_REQUEST, &mut self.c, &mut self.m)
            .map(|_| ())
    }

    /// Result word and raw reply bytes (no handles in the reply).
    fn reply(&self, length: usize) -> (u32, Vec<u8>) {
        let mut out = vec![0u8; length];
        self.m.read(0x2020, &mut out).unwrap();
        (self.m.read_u32(0x2018).unwrap(), out)
    }

    /// Open a 48 kHz decoder and make it the current session.
    fn open_decoder(&mut self, channels: u32) {
        let size = OpusParameters {
            sample_rate: 48_000,
            channel_count: channels,
            large_frames: false,
        }
        .work_buffer_size();
        self.request(0, Some(self.tmem), &[48_000, channels, size]).unwrap();
        let decoder = self.m.read_u32(0x200c).unwrap();
        self.session = decoder;
    }

    /// Write `header + packet` at `INPUT`; returns the A descriptor.
    fn put_packet(&mut self, packet: &[u8], final_range: u32) -> (u64, u64) {
        let mut bytes = (packet.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&final_range.to_be_bytes());
        bytes.extend_from_slice(packet);
        self.m.write(INPUT, &bytes).unwrap();
        (INPUT, bytes.len() as u64)
    }

    fn pcm(&self, samples: usize) -> Vec<i16> {
        let mut bytes = vec![0u8; samples * 2];
        self.m.read(OUTPUT, &mut bytes).unwrap();
        bytes.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
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
fn open_retains_transfer_memory_and_unimplemented_commands_fail_explicitly() {
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
    // SetContext and the multi-stream decode commands are not implemented.
    for command in [1, 2, 3, 5, 7, 9] {
        assert!(matches!(
            r.request(command, None, &[]),
            Err(KernelError::Service(ServiceError::UnsupportedCommand {
                service: "IHardwareOpusDecoder",
                ..
            }))
        ));
    }
    // Decode commands need exactly one A and one B buffer.
    assert!(matches!(
        r.request(0, None, &[]),
        Err(KernelError::Service(ServiceError::InvalidBufferCounts { .. }))
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

/// A CELT-only packet (config 31: fullband, 20 ms, stereo, one frame) with
/// arbitrary payload: every byte string is a decodable CELT frame.
fn celt_packet(seed: u8) -> Vec<u8> {
    let mut p = vec![0xfc];
    p.extend((0..80u8).map(|i| i.wrapping_mul(37).wrapping_add(seed)));
    p
}

fn reference(packets: &[Vec<u8>]) -> (Vec<i16>, u32) {
    let mut d = Decoder::new(48_000, 2).unwrap();
    let mut pcm = vec![0i16; 960 * 2];
    for p in packets {
        d.decode(Some(p), &mut pcm, 960).unwrap();
    }
    (pcm, d.final_range())
}

#[test]
fn decode_interleaved_writes_pcm_and_reports_sizes() {
    let mut r = Rig::new();
    r.open_decoder(2);
    let packet = celt_packet(1);
    let (want, range) = reference(std::slice::from_ref(&packet));
    // DecodeInterleaved (8): reset flag in, {consumed, samples, time} out.
    let a = r.put_packet(&packet, range);
    r.buffers(8, &[a], &[(OUTPUT, OUTPUT_SIZE)], &[0]).unwrap();
    let (result, out) = r.reply(16);
    assert_eq!(result, 0);
    assert_eq!(u32::from_le_bytes(out[..4].try_into().unwrap()), packet.len() as u32 + 8);
    assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 960);
    let micros = u64::from_le_bytes(out[8..16].try_into().unwrap());
    assert_eq!(micros, 200, "modelled cost: 1% of 20 ms");
    assert_eq!(r.pcm(960 * 2), want);
    // DecodeInterleavedOld (0) continues the same stream state.
    let second = celt_packet(2);
    let (want, range) = reference(&[packet.clone(), second.clone()]);
    let a = r.put_packet(&second, range);
    r.buffers(0, &[a], &[(OUTPUT, OUTPUT_SIZE)], &[]).unwrap();
    let (result, out) = r.reply(8);
    assert_eq!(result, 0);
    assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 960);
    assert_eq!(r.pcm(960 * 2), want);
}

#[test]
fn reset_flag_restarts_the_stream() {
    let mut r = Rig::new();
    r.open_decoder(2);
    let first = celt_packet(3);
    let second = celt_packet(4);
    let a = r.put_packet(&first, 0);
    r.buffers(4, &[a], &[(OUTPUT, OUTPUT_SIZE)], &[]).unwrap();
    assert_eq!(r.reply(16).0, 0);
    // With reset (6) the second packet decodes as if it were the first.
    let (want, range) = reference(std::slice::from_ref(&second));
    let a = r.put_packet(&second, range);
    r.buffers(6, &[a], &[(OUTPUT, OUTPUT_SIZE)], &[1]).unwrap();
    assert_eq!(r.reply(16).0, 0);
    assert_eq!(r.pcm(960 * 2), want);
}

#[test]
fn decode_errors_are_reported_with_hwopus_results() {
    let mut r = Rig::new();
    r.open_decoder(2);
    let packet = celt_packet(5);
    // Input must be larger than the 8-byte header.
    r.m.write(INPUT, &[0; 8]).unwrap();
    r.buffers(8, &[(INPUT, 8)], &[(OUTPUT, OUTPUT_SIZE)], &[0]).unwrap();
    assert_eq!(r.reply(4).0, RESULT_OPUS_INPUT_TOO_SMALL);
    // Header size beyond the input buffer.
    let (address, size) = r.put_packet(&packet, 0);
    r.buffers(8, &[(address, size - 1)], &[(OUTPUT, OUTPUT_SIZE)], &[0]).unwrap();
    assert_eq!(r.reply(4).0, RESULT_OPUS_BUFFER_TOO_SMALL);
    // Output buffer smaller than one 20 ms stereo frame.
    let a = r.put_packet(&packet, 0);
    r.buffers(8, &[a], &[(OUTPUT, 959 * 4)], &[0]).unwrap();
    assert_eq!(r.reply(4).0, RESULT_OPUS_BUFFER_TOO_SMALL);
    // Final range that does not match the decoded packet.
    let (_, range) = reference(std::slice::from_ref(&packet));
    let a = r.put_packet(&packet, range ^ 1);
    r.buffers(8, &[a], &[(OUTPUT, OUTPUT_SIZE)], &[1]).unwrap();
    assert_eq!(r.reply(4).0, RESULT_OPUS_INVALID_PACKET);
    // Malformed framing: code 1 (two CBR frames) with an odd payload.
    let a = r.put_packet(&[0xfd, 1, 2, 3], 0);
    r.buffers(8, &[a], &[(OUTPUT, OUTPUT_SIZE)], &[0]).unwrap();
    assert_eq!(r.reply(4).0, RESULT_OPUS_INVALID_PACKET);
}
