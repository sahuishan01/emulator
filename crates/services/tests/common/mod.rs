//! CMIF request rig shared by service tests: one named port, one session.
#![allow(dead_code)]
use emulator_a64::Cpu;
use emulator_kernel::service::Service;
use emulator_kernel::{Kernel, SVC_CONNECT_TO_NAMED_PORT, SVC_SEND_SYNC_REQUEST};
use emulator_memory::{AddressSpace, Permissions};

pub const TLS: u64 = 0x2000;

pub struct Rig {
    pub k: Kernel,
    pub c: Cpu,
    pub m: AddressSpace,
    pub session: u32,
}

impl Rig {
    /// Connects to `port`, served by `service`; map extra buffers via `m`.
    pub fn new(port: &str, service: Box<dyn Service>) -> Self {
        let mut k = Kernel::default();
        let mut c = Cpu::new(0, 0);
        let mut m = AddressSpace::default();
        let mut name = port.as_bytes().to_vec();
        name.push(0);
        m.map(0x1000, name, Permissions::READ).unwrap();
        m.map(TLS, vec![0; 256], Permissions::READ_WRITE).unwrap();
        k.register_named_port(port, service).unwrap();
        c.tpidrro_el0 = TLS;
        c.x[1] = 0x1000;
        k.dispatch(SVC_CONNECT_TO_NAMED_PORT, &mut c, &mut m).unwrap();
        let session = c.x[1] as u32;
        Self { k, c, m, session }
    }

    /// CMIF request: `a` send / `b` receive alias buffers, optional C buffer.
    pub fn call(&mut self, command: u32, a: &[(u64, u64)], b: &[(u64, u64)], c: Option<(u64, u64)>, data: &[u8]) {
        let mut p = [0u8; 256];
        let word0 = 4 | ((a.len() as u32) << 20) | ((b.len() as u32) << 24);
        let raw_words = 8 + data.len().div_ceil(4) as u32;
        let word1 = raw_words | if c.is_some() { 2 << 10 } else { 0 };
        p[..4].copy_from_slice(&word0.to_le_bytes());
        p[4..8].copy_from_slice(&word1.to_le_bytes());
        let mut at = 8;
        for &(address, size) in a.iter().chain(b) {
            for (i, v) in [size as u32, address as u32, 0].into_iter().enumerate() {
                p[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            at += 12;
        }
        let raw_start = at;
        let header = (at + 15) & !15;
        p[header..header + 4].copy_from_slice(b"SFCI");
        p[header + 8..header + 12].copy_from_slice(&command.to_le_bytes());
        p[header + 16..header + 16 + data.len()].copy_from_slice(data);
        if let Some((address, size)) = c {
            let list = raw_start + raw_words as usize * 4;
            p[list..list + 4].copy_from_slice(&(address as u32).to_le_bytes());
            let fields = ((address >> 32) as u32 & 0xffff) | ((size as u32) << 16);
            p[list + 4..list + 8].copy_from_slice(&fields.to_le_bytes());
        }
        self.m.write(TLS, &p).unwrap();
        self.c.x[0] = u64::from(self.session);
        self.k.dispatch(SVC_SEND_SYNC_REQUEST, &mut self.c, &mut self.m).unwrap();
    }

    pub fn result(&self) -> u32 {
        self.m.read_u32(TLS + 0x18).unwrap()
    }
    pub fn out(&self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; length];
        self.m.read(TLS + 0x20, &mut bytes).unwrap();
        bytes
    }
}

