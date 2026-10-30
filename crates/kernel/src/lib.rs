//! Minimal original Horizon SVC subset; no scheduler or service-success stubs.

use emulator_a64::Cpu;
use emulator_memory::{AddressSpace, MemoryError};
use std::fmt;

pub const SVC_EXIT_PROCESS: u16 = 0x07;
pub const SVC_OUTPUT_DEBUG_STRING: u16 = 0x27;
pub const MAIN_THREAD_HANDLE: u64 = 0x100;
const MAX_DEBUG_MESSAGE: usize = 16 * 1024;
const MAX_DEBUG_BYTES: usize = 1024 * 1024;
const MAX_DEBUG_RECORDS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelError {
    ProcessAlreadyExited,
    UnsupportedSupervisorCall(u16),
    InvalidDebugBuffer(MemoryError),
    DebugLimit,
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProcessAlreadyExited => write!(f, "guest process has already exited"),
            Self::UnsupportedSupervisorCall(id) => write!(f, "unsupported guest SVC {id:#x}"),
            Self::InvalidDebugBuffer(error) => write!(f, "invalid guest debug buffer: {error}"),
            Self::DebugLimit => write!(f, "guest debug-output limit exceeded"),
        }
    }
}
impl std::error::Error for KernelError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelEvent {
    Continue,
    ProcessExited,
}

#[derive(Debug, Default)]
pub struct Kernel {
    debug_records: Vec<Vec<u8>>,
    debug_bytes: usize,
    exited: bool,
}

impl Kernel {
    pub fn debug_records(&self) -> &[Vec<u8>] {
        &self.debug_records
    }
    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn dispatch(
        &mut self,
        id: u16,
        cpu: &mut Cpu,
        memory: &AddressSpace,
    ) -> Result<KernelEvent, KernelError> {
        if self.exited {
            return Err(KernelError::ProcessAlreadyExited);
        }
        match id {
            SVC_EXIT_PROCESS => {
                self.exited = true;
                Ok(KernelEvent::ProcessExited)
            }
            SVC_OUTPUT_DEBUG_STRING => {
                let size = usize::try_from(cpu.x[1]).map_err(|_| KernelError::DebugLimit)?;
                if size > MAX_DEBUG_MESSAGE
                    || self.debug_bytes + size > MAX_DEBUG_BYTES
                    || self.debug_records.len() >= MAX_DEBUG_RECORDS
                {
                    return Err(KernelError::DebugLimit);
                }
                let mut bytes = vec![0; size];
                memory
                    .read(cpu.x[0], &mut bytes)
                    .map_err(KernelError::InvalidDebugBuffer)?;
                self.debug_records.push(bytes);
                self.debug_bytes += size;
                cpu.x[0] = 0;
                Ok(KernelEvent::Continue)
            }
            _ => Err(KernelError::UnsupportedSupervisorCall(id)),
        }
    }
}

/// Supported entry hints only; this does not advertise a full homebrew runtime.
pub fn homebrew_config() -> Vec<u8> {
    let available = (1_u64 << SVC_EXIT_PROCESS) | (1_u64 << SVC_OUTPUT_DEBUG_STRING);
    let entries = [
        (1_u32, MAIN_THREAD_HANDLE, 0_u64),
        (7, 0, 0),
        (6, available, 0),
        (0, 0, 0),
    ];
    let mut bytes = Vec::new();
    for (key, value0, value1) in entries {
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&value0.to_le_bytes());
        bytes.extend_from_slice(&value1.to_le_bytes());
    }
    bytes
}
