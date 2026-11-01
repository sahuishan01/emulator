//! Integrated protocol cases run the actual kernel/NV stack with owned fixtures.
//! These are host API regressions, separate from the A64 guest suite.
use crate::{
    nvas::BIND_CHANNEL,
    nvchannel::{ALLOC_CONTEXT, ALLOC_FIFO, SET_NVMAP, SYNC_READ, SYNC_WAIT},
    NvDriver,
};
use emulator_a64::Cpu;
use emulator_ipc::Request;
use emulator_kernel::{
    service::{Service, ServiceContext, ServiceError, ServiceReply},
    Kernel, KernelError, CURRENT_PROCESS_HANDLE, SVC_CLOSE_HANDLE, SVC_CONNECT_TO_NAMED_PORT,
    SVC_CREATE_TRANSFER_MEMORY, SVC_EXIT_PROCESS, SVC_SEND_SYNC_REQUEST,
};
use emulator_memory::{AddressSpace, Permissions};
use std::panic::{catch_unwind, AssertUnwindSafe};

const INIT: u32 = 0x40284109;
const MAP: u32 = 0xc0284106;
const SUBMIT: u32 = 0xc0204808;
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn packet(command: u32, data: &[u32], a: Option<(u32, u32)>, b: Option<(u32, u32)>) -> [u8; 256] {
    let mut bytes = [0; 256];
    bytes[..4].copy_from_slice(
        &(4 | (u32::from(a.is_some()) << 20) | (u32::from(b.is_some()) << 24)).to_le_bytes(),
    );
    bytes[4..8].copy_from_slice(&(8 + data.len() as u32).to_le_bytes());
    let mut at = 8;
    for (address, size) in [a, b].into_iter().flatten() {
        bytes[at..at + 4].copy_from_slice(&size.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&address.to_le_bytes());
        at += 12
    }
    let cmif = (at + 15) & !15;
    bytes[cmif..cmif + 4].copy_from_slice(b"SFCI");
    bytes[cmif + 8..cmif + 12].copy_from_slice(&command.to_le_bytes());
    bytes[cmif + 16..cmif + 16 + data.len() * 4].copy_from_slice(&words(data));
    bytes
}
#[derive(Clone, Debug, Default)]
struct ReviewDriver(NvDriver);
impl Service for ReviewDriver {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, r: &Request) -> Result<ServiceReply, ServiceError> {
        self.0.dispatch(r)
    }
    fn prepare(
        &mut self,
        r: &Request,
        c: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        let mut request = r.clone();
        if request.command_id >= 100 {
            request.command_id -= 100
        }
        let mut reply = self.0.prepare(&request, c)?;
        if r.command_id >= 100 {
            reply.data = vec![0; 300]
        }
        Ok(reply)
    }
}
struct Rig {
    k: Kernel,
    c: Cpu,
    m: AddressSpace,
    session: u32,
    nv: u32,
    asfd: u32,
    channel: u32,
    ctrl: u32,
    point: u32,
    verified: usize,
}
impl Rig {
    fn new() -> Self {
        let (mut k, mut c, mut m) = (Kernel::default(), Cpu::new(0, 0), AddressSpace::default());
        for (address, bytes, perm) in [
            (0x1000, b"nv:\0".to_vec(), Permissions::READ),
            (0x2000, vec![0; 4096], Permissions::READ_WRITE),
            (0x4000, vec![0; 4096], Permissions::READ_WRITE),
            (0x6000, vec![0; 4096], Permissions::READ_WRITE),
            (0x8000, vec![0; 4096], Permissions::READ_WRITE),
        ] {
            m.map(address, bytes, perm).unwrap()
        }
        m.write_u64(0x4000, 42).unwrap();
        c.x[1] = 0x4000;
        c.x[2] = 4096;
        c.x[3] = 0;
        k.dispatch(SVC_CREATE_TRANSFER_MEMORY, &mut c, &mut m)
            .unwrap();
        let tmem = c.x[1] as u32;
        k.register_named_port("nv:", Box::new(ReviewDriver::default()))
            .unwrap();
        c.tpidrro_el0 = 0x2000;
        c.x[1] = 0x1000;
        k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m)
            .unwrap();
        let session = c.x[1] as u32;
        let mut r = Self {
            k,
            c,
            m,
            session,
            nv: 0,
            asfd: 0,
            channel: 0,
            ctrl: 0,
            point: 0,
            verified: 0,
        };
        let mut init = [0; 256];
        init[..4].copy_from_slice(&4_u32.to_le_bytes());
        init[4..8].copy_from_slice(&0x80000009_u32.to_le_bytes());
        init[8..12].copy_from_slice(&4_u32.to_le_bytes());
        init[12..16].copy_from_slice(&CURRENT_PROCESS_HANDLE.to_le_bytes());
        init[16..20].copy_from_slice(&tmem.to_le_bytes());
        init[32..36].copy_from_slice(b"SFCI");
        init[40..44].copy_from_slice(&3_u32.to_le_bytes());
        init[48..52].copy_from_slice(&4096_u32.to_le_bytes());
        r.call(&init).unwrap();
        r.close_handle(tmem);
        r
    }
    fn call(&mut self, bytes: &[u8]) -> Result<(), KernelError> {
        self.m.write(0x2000, bytes).unwrap();
        self.c.x[0] = u64::from(self.session);
        self.k
            .dispatch(SVC_SEND_SYNC_REQUEST, &mut self.c, &mut self.m)
            .map(|_| ())
    }
    fn close_handle(&mut self, handle: u32) {
        self.c.x[0] = u64::from(handle);
        self.k
            .dispatch(SVC_CLOSE_HANDLE, &mut self.c, &mut self.m)
            .unwrap();
    }
    fn open_at(&mut self, path: &[u8], command: u32) -> Result<(u32, u32), KernelError> {
        self.m.write(0x6000, path).unwrap();
        self.call(&packet(
            command,
            &[],
            Some((0x6000, path.len() as u32)),
            None,
        ))?;
        Ok((
            self.m.read_u32(0x2020).unwrap(),
            self.m.read_u32(0x2024).unwrap(),
        ))
    }
    fn open(&mut self, path: &[u8]) -> u32 {
        let (fd, error) = self.open_at(path, 0).unwrap();
        assert_eq!(error, 0);
        assert_ne!(fd, 0);
        fd
    }
    fn io_at(
        &mut self,
        fd: u32,
        code: u32,
        args: &[u32],
        command: u32,
        b_address: u32,
    ) -> Result<u32, KernelError> {
        let n = args.len() as u32 * 4;
        self.m.write(0x6000, &words(args)).unwrap();
        self.call(&packet(
            command,
            &[fd, code],
            if code & (1 << 30) != 0 {
                Some((0x6000, n))
            } else {
                None
            },
            if code & (1 << 31) != 0 {
                Some((b_address, n))
            } else {
                None
            },
        ))?;
        Ok(self.m.read_u32(0x2020).unwrap())
    }
    fn io(&mut self, fd: u32, code: u32, args: &[u32]) -> u32 {
        self.io_at(fd, code, args, 1, 0x6000).unwrap()
    }
    fn close(&mut self, fd: u32) -> u32 {
        self.call(&packet(2, &[fd], None, None)).unwrap();
        self.m.read_u32(0x2020).unwrap()
    }
    fn read(&mut self, point: u32) -> u32 {
        assert_eq!(self.io(self.ctrl, SYNC_READ, &[point, 0]), 0);
        self.m.read_u32(0x6004).unwrap()
    }
    fn program(&mut self, data: &[u32]) {
        self.m.write(0x8000, &words(data)).unwrap()
    }
    fn full_program(&mut self) {
        self.program(&[
            0x20010000, 0xb0b5, 0x20010100, 1, 0x20010101, 0x100, 0x20010102, 1, 0x20010103, 0x108,
            0x20010106, 8, 0x200100c0, 0x182,
        ]);
    }
    fn submit(&mut self, flags: u32, id: u32, value: u32, entry: u32, length: u32) -> u32 {
        self.io(
            self.channel,
            SUBMIT,
            &[0, 0, 1, flags, id, value, entry, (length << 10) | 1],
        )
    }
    fn verify(&mut self, address: u64, value: u64) {
        let actual = self.m.read_u64(address).unwrap();
        assert_eq!(actual, value);
        self.verified += 8
    }
    fn prepare(&mut self, write: bool, context: bool) {
        self.nv = self.open(b"/dev/nvmap");
        assert_eq!(self.io(self.nv, 0xc0080101, &[4096, 0]), 0);
        let handle = self.m.read_u32(0x6004).unwrap();
        assert_eq!(
            self.io(
                self.nv,
                0xc0200104,
                &[handle, 0, u32::from(write), 4096, 0, 0, 0x8000, 0]
            ),
            0
        );
        self.asfd = self.open(b"/dev/nvhost-as-gpu");
        assert_eq!(self.io(self.asfd, INIT, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0]), 0);
        assert_eq!(
            self.io(self.asfd, MAP, &[0, 0, handle, 0, 0, 0, 0, 0, 0, 0]),
            0
        );
        assert_eq!(self.m.read_u64(0x6020).unwrap(), 1 << 32);
        self.channel = self.open(b"/dev/nvhost-gpu");
        assert_eq!(self.io(self.channel, SET_NVMAP, &[self.nv]), 0);
        assert_eq!(self.io(self.asfd, BIND_CHANNEL, &[self.channel]), 0);
        if context {
            assert_eq!(self.io(self.channel, ALLOC_CONTEXT, &[0xb0b5, 0, 0, 0]), 0)
        }
        assert_eq!(
            self.io(self.channel, ALLOC_FIFO, &[8, 0, 1, 0, 0, 0, 0, 0]),
            0
        );
        self.point = self.m.read_u32(0x600c).unwrap();
        assert_ne!(self.point, 0);
        assert_eq!(self.m.read_u32(0x6010).unwrap(), 0);
        self.ctrl = self.open(b"/dev/nvhost-ctrl");
        self.m.write_u64(0x8100, 42).unwrap();
        self.m.write_u64(0x8108, 99).unwrap();
        self.m.write(0x8080, &words(&[0x200100c0, 0x182])).unwrap();
        self.full_program();
    }
    fn prepare_color(&mut self, write: bool) {
        self.prepare(write, false);
        assert_eq!(self.io(self.channel, ALLOC_CONTEXT, &[0xb197, 0, 0, 0]), 0);
        self.m.write(0x8800, &[0x55; 512]).unwrap();
    }
    fn color_program(&mut self, format: u32, extras: &[(u32, u32)]) -> u32 {
        let mut methods = vec![
            (0, 0xb197),
            (0x800, 1),
            (0x804, 0x800),
            (0x808, 64),
            (0x80c, 8),
            (0x810, format),
            (0x814, 0x1000),
            (0x818, 1),
            (0x81c, 0),
            (0x820, 0),
            (0xff4, 16 << 16),
            (0xff8, 8 << 16),
            (0x10f8, 0),
            (0xd80, 1.0_f32.to_bits()),
            (0xd84, 0),
            (0xd88, 0),
            (0xd8c, 1.0_f32.to_bits()),
        ];
        methods.extend_from_slice(extras);
        let words: Vec<_> = methods
            .iter()
            .flat_map(|(offset, data)| [0x20010000 | (offset / 4), *data])
            .collect();
        self.program(&words);
        words.len() as u32
    }
    fn verify_image(&mut self, expected: impl Fn(u32, u32) -> [u8; 4]) {
        let mut actual = [0; 512];
        self.m.read(0x8800, &mut actual).unwrap();
        for y in 0..8 {
            for x in 0..16 {
                assert_eq!(
                    &actual[((y * 16 + x) * 4) as usize..((y * 16 + x) * 4 + 4) as usize],
                    &expected(x, y),
                    "pixel {x},{y}"
                );
            }
        }
        self.verified += actual.len();
    }
    fn clone_client(&mut self) {
        let mut message = packet(2, &[], None, None);
        message[..4].copy_from_slice(&5_u32.to_le_bytes());
        self.call(&message).unwrap();
        let clone = self.m.read_u32(0x200c).unwrap();
        self.close_handle(self.session);
        self.session = clone;
    }
    fn finish(&mut self) {
        self.k
            .dispatch(SVC_EXIT_PROCESS, &mut self.c, &mut self.m)
            .unwrap();
        assert_eq!(self.k.live_handle_count(), 0);
        assert_eq!(self.k.live_transfer_memory_count(), 0);
        assert_eq!(self.m.lent_bytes(), 0);
        assert_eq!(self.m.read_u64(0x4000).unwrap(), 42);
    }
}
/// Actual case outcomes; no external inputs or commercial content are loaded.
#[derive(Debug)]
pub struct CaseResult {
    pub name: &'static str,
    pub passed: bool,
    pub ipc_calls: usize,
    pub verified_bytes: usize,
    pub handles_after_exit: Option<usize>,
    pub error: String,
}
/// Run independent protocol scenarios, including deliberately rejected requests.
pub fn run() -> Vec<CaseResult> {
    type Case = (&'static str, fn(&mut Rig));
    let cases: &[Case] = &[
        ("channel-setup-and-binding", |r| {
            r.prepare(true, true);
            assert_eq!(r.read(r.point), 0);
            assert_eq!(r.io(r.asfd, BIND_CHANNEL, &[r.channel]), 8);
            assert_eq!(r.io(r.channel, SET_NVMAP, &[r.nv]), 8);
            assert_eq!(r.io(r.channel, ALLOC_FIFO, &[8, 0, 1, 0, 0, 0, 0, 0]), 8);
        }),
        ("dma-submit-and-fence-read", |r| {
            r.prepare(true, true);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
            assert_eq!(r.m.read_u32(0x6014).unwrap(), 1);
            assert_eq!(r.read(r.point), 1);
        }),
        ("cloned-client-preserves-channel-state", |r| {
            r.prepare(true, true);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
            r.clone_client();
            r.m.write_u64(0x8100, 43).unwrap();
            assert_eq!(r.submit(2, 0, 0, 0x80, 2), 0);
            r.verify(0x8108, 43);
            assert_eq!(r.read(r.point), 2);
        }),
        ("rejected-commands-are-atomic", |r| {
            r.prepare(true, true);
            r.m.write(0x8038, &words(&[0x20010011, 0])).unwrap();
            assert_eq!(r.submit(2, 0, 0, 0, 16), 4);
            r.verify(0x8108, 99);
            assert_eq!(r.read(r.point), 0);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
            assert_eq!(r.read(r.point), 1);
        }),
        ("readonly-destination-rejected", |r| {
            r.prepare(false, true);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 4);
            r.verify(0x8108, 99);
            assert_eq!(r.read(r.point), 0);
        }),
        ("malformed-entries-and-capacity", |r| {
            r.prepare(true, true);
            assert_eq!(
                r.io(r.channel, SUBMIT, &[0, 0, 2, 2, 0, 0, 0, (14 << 10) | 1]),
                4
            );
            assert_eq!(
                r.io(
                    r.channel,
                    SUBMIT,
                    &[0, 0, 1, 2, 0, 0, 0, (14 << 10) | 0x101]
                ),
                4
            );
            let mut args = vec![0, 0, 9, 2, 0, 0];
            args.extend([0, 0].repeat(9));
            assert_eq!(r.io(r.channel, 0xc0604808, &args), 4);
            assert_eq!(r.submit(4, 0, 0, 0, 14), 2);
            r.verify(0x8108, 99);
            assert_eq!(r.read(r.point), 0);
        }),
        ("unsupported-context-and-recovery", |r| {
            r.prepare(true, false);
            assert_eq!(r.io(r.channel, ALLOC_CONTEXT, &[0xc197, 0, 0, 0]), 2);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 8);
            assert_eq!(r.io(r.channel, ALLOC_CONTEXT, &[0xb0b5, 0, 0, 0]), 0);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
        }),
        ("fence-wait-and-poll-thresholds", |r| {
            r.prepare(true, true);
            assert_eq!(r.submit(3, r.point, 1, 0, 14), 5);
            assert_eq!(r.io(r.ctrl, SYNC_WAIT, &[r.point, 1, 0, 99]), 5);
            assert_eq!(r.read(r.point), 0);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            assert_eq!(r.io(r.ctrl, SYNC_WAIT, &[r.point, 1, 0, 99]), 0);
            assert_eq!(r.m.read_u32(0x600c).unwrap(), 1);
            assert_eq!(r.io(r.ctrl, SYNC_WAIT, &[r.point, 2, 10, 99]), 2);
            assert_eq!(r.submit(3, r.point, 1, 0x80, 2), 0);
            r.verify(0x8108, 42);
            assert_eq!(r.read(r.point), 2);
        }),
        ("failed-reply-rolls-back-job-state-and-fence", |r| {
            r.prepare(true, true);
            let args = [0, 0, 1, 2, 0, 0, 0, (14 << 10) | 1];
            assert!(r.io_at(r.channel, SUBMIT, &args, 101, 0x6000).is_err());
            r.verify(0x8108, 99);
            assert_eq!(r.read(r.point), 0);
            assert_eq!(r.submit(2, 0, 0, 0x80, 2), 4);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
            assert_eq!(r.read(r.point), 1);
        }),
        ("ioctl-output-dma-alias-rejected", |r| {
            r.prepare(true, true);
            let before = r.m.read_u64(0x8108).unwrap();
            assert_eq!(
                r.io_at(
                    r.channel,
                    SUBMIT,
                    &[0, 0, 1, 2, 0, 0, 0, (14 << 10) | 1],
                    1,
                    0x8108
                )
                .unwrap(),
                4
            );
            r.verify(0x8108, before);
            assert_eq!(r.read(r.point), 0);
        }),
        ("dma-tls-alias-rejected-before-commit", |r| {
            r.prepare(true, true);
            assert_eq!(r.io(r.nv, 0xc0080101, &[4096, 0]), 0);
            let h = r.m.read_u32(0x6004).unwrap();
            assert_eq!(r.io(r.nv, 0xc0200104, &[h, 0, 1, 4096, 0, 0, 0x2000, 0]), 0);
            assert_eq!(r.io(r.asfd, MAP, &[0, 0, h, 0, 0, 0, 0, 0, 0, 0]), 0);
            r.m.write_u32(0x8024, 0x1000).unwrap();
            assert!(r
                .io_at(
                    r.channel,
                    SUBMIT,
                    &[0, 0, 1, 2, 0, 0, 0, (14 << 10) | 1],
                    1,
                    0x6000
                )
                .is_err());
            assert_eq!(r.read(r.point), 0);
            r.verify(0x8108, 99);
        }),
        ("detached-address-space-still-submits", |r| {
            r.prepare(true, true);
            assert_eq!(r.close(r.nv), 0);
            assert_eq!(r.close(r.asfd), 0);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
            assert_eq!(r.read(r.point), 1);
        }),
        ("cpu-reuse-does-not-redirect-channel", |r| {
            r.prepare(true, true);
            let view = r.m.pin(0x8000, 4096, Permissions::READ_WRITE).unwrap();
            r.m.map_alias(0xa000, &view, Permissions::READ_WRITE)
                .unwrap();
            drop(view);
            assert_eq!(r.close(r.nv), 0);
            assert_eq!(r.close(r.asfd), 0);
            r.m.unmap(0x8000, 4096).unwrap();
            r.m.map(0x8000, vec![99; 4096], Permissions::READ_WRITE)
                .unwrap();
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0xa108, 42);
            r.verify(0x8108, 0x6363636363636363);
            r.m.unmap(0xa000, 4096).unwrap();
            let resident = r.m.resident_bytes();
            assert_eq!(r.close(r.channel), 0);
            assert_eq!(r.m.resident_bytes(), resident - 4096);
            assert_eq!(r.read(r.point), 1);
        }),
        ("final-channel-close-releases-backing", |r| {
            r.prepare(true, true);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
            assert_eq!(r.close(r.nv), 0);
            assert_eq!(r.close(r.asfd), 0);
            let before = r.m.resident_bytes();
            r.m.unmap(0x8000, 4096).unwrap();
            assert_eq!(r.m.resident_bytes(), before);
            assert_eq!(r.close(r.channel), 0);
            assert_eq!(r.m.resident_bytes(), before - 4096);
            assert_eq!(r.read(r.point), 1);
        }),
        ("channel-state-and-syncpoints-isolated", |r| {
            r.prepare(true, true);
            let first = r.channel;
            let second = r.open(b"/dev/nvhost-gpu");
            assert_eq!(r.io(second, SET_NVMAP, &[r.nv]), 0);
            assert_eq!(r.io(r.asfd, BIND_CHANNEL, &[second]), 0);
            assert_eq!(r.io(second, ALLOC_CONTEXT, &[0xb0b5, 0, 0, 0]), 0);
            assert_eq!(r.io(second, ALLOC_FIFO, &[8, 0, 1, 0, 0, 0, 0, 0]), 0);
            let point2 = r.m.read_u32(0x600c).unwrap();
            assert_ne!(point2, r.point);
            r.channel = second;
            assert_eq!(r.submit(2, 0, 0, 0x80, 2), 4);
            r.channel = first;
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 42);
            r.m.write_u64(0x8100, 43).unwrap();
            r.channel = second;
            assert_eq!(r.submit(2, 0, 0, 0, 14), 0);
            r.verify(0x8108, 43);
            assert_eq!(r.read(r.point), 1);
            assert_eq!(r.read(point2), 1);
        }),
        ("syncpoint-lifetime-quota-and-stale-fds", |r| {
            r.prepare(true, true);
            for _ in 1..32 {
                let fd = r.open(b"/dev/nvhost-gpu");
                assert_eq!(r.close(fd), 0);
            }
            assert_eq!(r.open_at(b"/dev/nvhost-gpu", 0).unwrap(), (0, 6));
            assert_eq!(r.close(r.channel), 0);
            assert_eq!(r.submit(2, 0, 0, 0, 14), 4);
            assert_eq!(r.read(r.point), 0);
            assert_eq!(r.io(r.ctrl, SYNC_READ, &[33, 0]), 4);
        }),
        ("prerequisites-and-rebinding-errors", |r| {
            r.nv = r.open(b"/dev/nvmap");
            r.asfd = r.open(b"/dev/nvhost-as-gpu");
            r.channel = r.open(b"/dev/nvhost-gpu");
            assert_eq!(r.io(r.channel, ALLOC_FIFO, &[8, 0, 1, 0, 0, 0, 0, 0]), 3);
            assert_eq!(r.io(r.asfd, INIT, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0]), 0);
            assert_eq!(r.io(r.asfd, BIND_CHANNEL, &[r.channel]), 3);
            assert_eq!(r.io(r.channel, SET_NVMAP, &[r.asfd]), 4);
            assert_eq!(r.io(r.channel, SET_NVMAP, &[r.nv]), 0);
            assert_eq!(r.io(r.asfd, BIND_CHANNEL, &[r.nv]), 4);
            assert_eq!(r.io(r.asfd, BIND_CHANNEL, &[r.channel]), 0);
            assert_eq!(r.io(r.channel, ALLOC_FIFO, &[3, 0, 1, 0, 0, 0, 0, 0]), 4);
        }),
        ("open-and-setup-reply-rollback", |r| {
            assert!(r.open_at(b"/dev/nvhost-gpu", 100).is_err());
            r.nv = r.open(b"/dev/nvmap");
            assert_eq!(r.nv, 1);
            r.asfd = r.open(b"/dev/nvhost-as-gpu");
            assert_eq!(r.io(r.asfd, INIT, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0]), 0);
            r.channel = r.open(b"/dev/nvhost-gpu");
            assert!(r.io_at(r.channel, SET_NVMAP, &[r.nv], 101, 0x6000).is_err());
            assert_eq!(r.io(r.asfd, BIND_CHANNEL, &[r.channel]), 3);
            assert_eq!(r.io(r.channel, SET_NVMAP, &[r.nv]), 0);
            assert!(r
                .io_at(r.asfd, BIND_CHANNEL, &[r.channel], 101, 0x6000)
                .is_err());
            assert_eq!(r.io(r.asfd, BIND_CHANNEL, &[r.channel]), 0);
            assert!(r
                .io_at(
                    r.channel,
                    ALLOC_FIFO,
                    &[8, 0, 1, 0, 0, 0, 0, 0],
                    101,
                    0x6000
                )
                .is_err());
            assert_eq!(r.io(r.channel, ALLOC_FIFO, &[8, 0, 1, 0, 0, 0, 0, 0]), 0);
            assert_eq!(r.m.read_u32(0x600c).unwrap(), 1);
        }),
        ("color-rgba-completion-and-readback", |r| {
            r.prepare_color(true);
            let n = r.color_program(0xd5, &[(0x19d0, 0x3c)]);
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|_, _| [255, 0, 0, 255]);
            assert_eq!(r.read(r.point), 1);
        }),
        ("color-bgra-storage-order", |r| {
            r.prepare_color(true);
            let n = r.color_program(0xcf, &[(0x19d0, 0x3c)]);
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|_, _| [0, 0, 255, 255]);
        }),
        ("color-scissor-preserves-outside", |r| {
            r.prepare_color(true);
            let n = r.color_program(
                0xd5,
                &[
                    (0xe00, 1),
                    (0xe04, (12 << 16) | 4),
                    (0xe08, (6 << 16) | 2),
                    (0x10f8, 0x100),
                    (0x19d0, 0x3c),
                ],
            );
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|x, y| {
                if (4..12).contains(&x) && (2..6).contains(&y) {
                    [255, 0, 0, 255]
                } else {
                    [0x55; 4]
                }
            });
        }),
        ("color-clear-rect-and-scissor-intersection", |r| {
            r.prepare_color(true);
            let n = r.color_program(
                0xd5,
                &[
                    (0xd6c, (10 << 16) | 2),
                    (0xd70, (5 << 16) | 1),
                    (0xe00, 1),
                    (0xe04, (15 << 16) | 7),
                    (0xe08, (8 << 16) | 3),
                    (0x10f8, 0x110),
                    (0x19d0, 0x3c),
                ],
            );
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|x, y| {
                if (7..10).contains(&x) && (3..5).contains(&y) {
                    [255, 0, 0, 255]
                } else {
                    [0x55; 4]
                }
            });
        }),
        ("color-component-mask-and-ordered-overwrite", |r| {
            r.prepare_color(true);
            let n = r.color_program(
                0xcf,
                &[
                    (0x19d0, 0x3c),
                    (0xd84, 1.0_f32.to_bits()),
                    (0xd80, 0),
                    (0x19d0, 0x08),
                ],
            );
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|_, _| [0, 255, 255, 255]);
            assert_eq!(r.read(r.point), 1);
        }),
        ("color-finite-unorm-clamp", |r| {
            r.prepare_color(true);
            let n = r.color_program(
                0xd5,
                &[
                    (0xd80, (-1.0_f32).to_bits()),
                    (0xd84, 2.0_f32.to_bits()),
                    (0xd88, 0.25_f32.to_bits()),
                    (0x19d0, 0x3c),
                ],
            );
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|_, _| [0, 255, 64, 255]);
        }),
        ("color-readonly-destination-rejected", |r| {
            r.prepare_color(false);
            let n = r.color_program(0xd5, &[(0x19d0, 0x3c)]);
            assert_eq!(r.submit(2, 0, 0, 0, n), 4);
            r.verify_image(|_, _| [0x55; 4]);
            assert_eq!(r.read(r.point), 0);
        }),
        ("color-unsupported-state-is-atomic", |r| {
            r.prepare_color(true);
            for invalid in [
                (0x810, 0xd6),
                (0x814, 0),
                (0x818, 2),
                (0x820, 1),
                (0x800, 256),
                (0x804, 0x801),
                (0x808, 60),
                (0x80c, 0),
                (0xd80, f32::NAN.to_bits()),
                (0x10f8, 1),
                (0x19d0, 1),
                (0x19d0, 0x7c),
                (0xe00, 2),
            ] {
                let mut extras = vec![invalid];
                if invalid.0 == 0xe00 {
                    extras.push((0x10f8, 0x100));
                }
                if invalid.0 != 0x19d0 {
                    extras.push((0x19d0, 0x3c));
                }
                let n = r.color_program(0xd5, &extras);
                assert_eq!(r.submit(2, 0, 0, 0, n), 4, "{invalid:?}");
                r.verify_image(|_, _| [0x55; 4]);
                assert_eq!(r.read(r.point), 0);
            }
        }),
        ("color-late-error-rolls-back-state-and-pixels", |r| {
            r.prepare_color(true);
            let n = r.color_program(0xd5, &[(0x19d0, 0x3c), (0x1234, 0)]);
            assert_eq!(r.submit(2, 0, 0, 0, n), 4);
            r.verify_image(|_, _| [0x55; 4]);
            r.program(&[0x20010000 | (0x19d0 / 4), 0x3c]);
            assert_eq!(r.submit(2, 0, 0, 0, 2), 4);
            assert_eq!(r.read(r.point), 0);
        }),
        ("color-failed-reply-cancels-write-and-fence", |r| {
            r.prepare_color(true);
            let n = r.color_program(0xd5, &[(0x19d0, 0x3c)]);
            assert!(r
                .io_at(
                    r.channel,
                    SUBMIT,
                    &[0, 0, 1, 2, 0, 0, 0, (n << 10) | 1],
                    101,
                    0x6000
                )
                .is_err());
            r.verify_image(|_, _| [0x55; 4]);
            assert_eq!(r.read(r.point), 0);
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|_, _| [255, 0, 0, 255]);
        }),
        ("color-output-alias-rejected-before-commit", |r| {
            r.prepare_color(true);
            let n = r.color_program(0xd5, &[(0x19d0, 0x3c)]);
            assert_eq!(
                r.io_at(
                    r.channel,
                    SUBMIT,
                    &[0, 0, 1, 2, 0, 0, 0, (n << 10) | 1],
                    1,
                    0x8800
                )
                .unwrap(),
                4
            );
            r.verify_image(|_, _| [0x55; 4]);
            assert_eq!(r.read(r.point), 0);
        }),
        ("color-closed-owners-and-cloned-client-retained", |r| {
            r.prepare_color(true);
            let n = r.color_program(0xd5, &[(0x19d0, 0x3c)]);
            assert_eq!(r.close(r.nv), 0);
            assert_eq!(r.close(r.asfd), 0);
            r.clone_client();
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|_, _| [255, 0, 0, 255]);
            assert_eq!(r.close(r.channel), 0);
            assert_eq!(r.read(r.point), 1);
        }),
        ("color-active-tls-alias-rejected-before-effects", |r| {
            r.prepare_color(true);
            assert_eq!(r.io(r.nv, 0xc0080101, &[4096, 0]), 0);
            let object = r.m.read_u32(0x6004).unwrap();
            assert_eq!(
                r.io(r.nv, 0xc0200104, &[object, 0, 1, 4096, 0, 0, 0x2000, 0]),
                0
            );
            assert_eq!(r.io(r.asfd, MAP, &[0, 0, object, 0, 0, 0, 0, 0, 0, 0]), 0);
            assert_eq!(r.m.read_u64(0x6020).unwrap(), (1 << 32) + 4096);
            let n = r.color_program(0xd5, &[(0x804, 0x1000), (0x19d0, 0x3c)]);
            assert!(r
                .io_at(
                    r.channel,
                    SUBMIT,
                    &[0, 0, 1, 2, 0, 0, 0, (n << 10) | 1],
                    1,
                    0x6000
                )
                .is_err());
            r.verify_image(|_, _| [0x55; 4]);
            assert_eq!(r.read(r.point), 0);
        }),
        ("color-padded-pitch-preserves-row-padding", |r| {
            r.prepare_color(true);
            r.m.write(0x8800, &[0x55; 640]).unwrap();
            let n = r.color_program(0xd5, &[(0x808, 80), (0x19d0, 0x3c)]);
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            let mut image = [0; 640];
            r.m.read(0x8800, &mut image).unwrap();
            for row in image.chunks_exact(80) {
                for p in row[..64].chunks_exact(4) {
                    assert_eq!(p, &[255, 0, 0, 255]);
                }
                assert_eq!(&row[64..], &[0x55; 16]);
            }
            r.verified += 640;
        }),
        ("color-context-class-mismatch-rejected", |r| {
            r.prepare_color(true);
            r.full_program();
            assert_eq!(r.submit(2, 0, 0, 0, 14), 4);
            assert_eq!(r.read(r.point), 0);
        }),
        ("color-empty-rect-completes-without-writes", |r| {
            r.prepare_color(true);
            let n = r.color_program(
                0xd5,
                &[
                    (0xd6c, (32 << 16) | 20),
                    (0xd70, (8 << 16) | 2),
                    (0x10f8, 0x10),
                    (0x19d0, 0x3c),
                ],
            );
            assert_eq!(r.submit(2, 0, 0, 0, n), 0);
            r.verify_image(|_, _| [0x55; 4]);
            assert_eq!(r.read(r.point), 1);
        }),
    ];
    cases
        .iter()
        .map(|(name, test)| {
            let setup = catch_unwind(Rig::new);
            let mut r = match setup {
                Ok(r) => r,
                Err(error) => {
                    return CaseResult {
                        name,
                        passed: false,
                        ipc_calls: 0,
                        verified_bytes: 0,
                        handles_after_exit: None,
                        error: panic_text(error),
                    }
                }
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                test(&mut r);
                r.finish();
            }));
            let passed = outcome.is_ok();
            let error = outcome.err().map(panic_text).unwrap_or_default();
            CaseResult {
                name,
                passed,
                ipc_calls: r.k.ipc_records().len(),
                verified_bytes: r.verified,
                handles_after_exit: passed.then(|| r.k.live_handle_count()),
                error,
            }
        })
        .collect()
}

fn panic_text(error: Box<dyn std::any::Any + Send>) -> String {
    error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "NV reference panic".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn integrated_nv_protocol_cases_pass() {
        let cases = super::run();
        assert_eq!(cases.len(), 34);
        for case in cases {
            assert!(case.passed, "{}: {}", case.name, case.error)
        }
    }
}
