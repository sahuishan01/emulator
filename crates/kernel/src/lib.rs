//! Original bounded Horizon SVC/handle/IPC subset. No scheduler or success stubs.

mod handles;
pub mod service;

use emulator_a64::Cpu;
use emulator_ipc::{encode_reply, parse_request, CommandType, IpcError, IPC_BUFFER_SIZE};
use emulator_memory::{Access, AddressSpace, MemoryError};
pub use handles::HandleError;
use handles::{HandleTable, Object};
use service::{Service, ServiceError};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

pub const SVC_EXIT_PROCESS: u16 = 0x07;
pub const SVC_OUTPUT_DEBUG_STRING: u16 = 0x27;
pub const SVC_CLOSE_HANDLE: u16 = 0x16;
pub const SVC_CONNECT_TO_NAMED_PORT: u16 = 0x1f;
pub const SVC_SEND_SYNC_REQUEST: u16 = 0x21;
pub const SVC_CREATE_EVENT: u16 = 0x45;
pub const SVC_SIGNAL_EVENT: u16 = 0x11;
pub const SVC_CLEAR_EVENT: u16 = 0x12;
pub const SVC_RESET_SIGNAL: u16 = 0x17;
pub const SVC_WAIT_SYNCHRONIZATION: u16 = 0x18;
pub const RESULT_INVALID_HANDLE: u64 = 0xe401;
pub const RESULT_INVALID_ADDRESS: u64 = 0xe601;
pub const RESULT_TIMEOUT: u64 = 0xea01;
pub const RESULT_TOO_MANY_HANDLES: u64 = 0xee01;
pub const RESULT_INVALID_STATE: u64 = 0xfa01;
pub const MAIN_THREAD_HANDLE: u64 = handles::MAIN_THREAD_HANDLE as u64;
const MAX_DEBUG_MESSAGE: usize = 16 * 1024;
const MAX_DEBUG_BYTES: usize = 1024 * 1024;
const MAX_DEBUG_RECORDS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelError {
    ProcessAlreadyExited,
    UnsupportedSupervisorCall(u16),
    InvalidDebugBuffer(MemoryError),
    DebugLimit,
    Memory(MemoryError),
    Ipc(IpcError),
    Service(ServiceError),
    Handle(HandleError),
    InvalidPortName,
    UnknownPort(String),
    DuplicatePort(String),
    InvalidThreadLocalRegion,
    UnsupportedControl(u32),
    IpcTraceLimit,
    UnsupportedWaitTimeout(i64),
    InvalidWaitCount(i32),
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProcessAlreadyExited => write!(f, "guest process has already exited"),
            Self::UnsupportedSupervisorCall(id) => write!(f, "unsupported guest SVC {id:#x}"),
            Self::InvalidDebugBuffer(error) => write!(f, "invalid guest debug buffer: {error}"),
            Self::DebugLimit => write!(f, "guest debug-output limit exceeded"),
            Self::Memory(error) => error.fmt(f),
            Self::Ipc(error) => error.fmt(f),
            Self::Service(error) => error.fmt(f),
            Self::Handle(error) => write!(f, "guest handle error: {error:?}"),
            Self::InvalidPortName => write!(f, "invalid or unbounded guest named-port name"),
            Self::UnknownPort(name) => write!(f, "unsupported guest named port {name:?}"),
            Self::DuplicatePort(name) => {
                write!(f, "guest named port {name:?} is already registered")
            }
            Self::InvalidThreadLocalRegion => {
                write!(f, "guest thread-local region must be 16-byte aligned")
            }
            Self::UnsupportedControl(id) => write!(f, "unsupported CMIF control command {id}"),
            Self::IpcTraceLimit => write!(f, "guest IPC trace limit exceeded"),
            Self::UnsupportedWaitTimeout(timeout) => write!(
                f,
                "blocking guest waits are not implemented (timeout {timeout})"
            ),
            Self::InvalidWaitCount(count) => write!(f, "invalid guest wait handle count {count}"),
        }
    }
}
impl std::error::Error for KernelError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelEvent {
    Continue,
    ProcessExited,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcRecord {
    pub handle: u32,
    pub kind: CommandType,
    pub command_id: u32,
    pub request_bytes: usize,
    pub reply_bytes: usize,
}

#[derive(Debug, Default)]
pub struct Kernel {
    debug_records: Vec<Vec<u8>>,
    debug_bytes: usize,
    exited: bool,
    handles: HandleTable,
    ports: BTreeMap<String, Box<dyn Service>>,
    ipc_records: Vec<IpcRecord>,
}

impl Kernel {
    pub fn debug_records(&self) -> &[Vec<u8>] {
        &self.debug_records
    }
    pub fn exited(&self) -> bool {
        self.exited
    }
    pub fn live_handle_count(&self) -> usize {
        self.handles.len()
    }
    pub fn ipc_records(&self) -> &[IpcRecord] {
        &self.ipc_records
    }

    pub fn register_named_port(
        &mut self,
        name: &str,
        service: Box<dyn Service>,
    ) -> Result<(), KernelError> {
        if name.is_empty()
            || name.len() > 11
            || !name.is_ascii()
            || name.contains('\0')
            || self.ports.len() >= 16
        {
            return Err(KernelError::InvalidPortName);
        }
        if self.ports.contains_key(name) {
            return Err(KernelError::DuplicatePort(name.into()));
        }
        self.ports.insert(name.into(), service);
        Ok(())
    }

    fn connect(&mut self, cpu: &mut Cpu, memory: &AddressSpace) -> Result<(), KernelError> {
        let mut name = Vec::new();
        let mut terminated = false;
        for offset in 0..12 {
            let address = cpu.x[1]
                .checked_add(offset)
                .ok_or(KernelError::Memory(MemoryError::AddressOverflow))?;
            let mut byte = [0];
            memory
                .read(address, &mut byte)
                .map_err(KernelError::Memory)?;
            if byte[0] == 0 {
                terminated = true;
                break;
            }
            name.push(byte[0]);
        }
        if !terminated || name.is_empty() || !name.is_ascii() {
            return Err(KernelError::InvalidPortName);
        }
        let name = String::from_utf8(name).expect("ASCII name");
        let service = self
            .ports
            .get(&name)
            .ok_or_else(|| KernelError::UnknownPort(name.clone()))?
            .clone_for_request();
        let pending = self
            .handles
            .stage(vec![Object::session(service)])
            .map_err(KernelError::Handle)?;
        let handle = pending.ids[0];
        self.handles.commit(pending);
        cpu.x[0] = 0;
        cpu.x[1] = u64::from(handle);
        Ok(())
    }

    fn send_sync(&mut self, cpu: &mut Cpu, memory: &mut AddressSpace) -> Result<(), KernelError> {
        let handle = cpu.x[0] as u32;
        let service = match self.handles.get(handle) {
            Ok(Object::Session(service)) => service,
            _ => {
                cpu.x[0] = RESULT_INVALID_HANDLE;
                return Ok(());
            }
        };
        if cpu.tpidrro_el0 % 16 != 0 {
            return Err(KernelError::InvalidThreadLocalRegion);
        }
        if self.ipc_records.len() >= 4096 {
            return Err(KernelError::IpcTraceLimit);
        }
        let mut input = [0; IPC_BUFFER_SIZE];
        memory
            .read(cpu.tpidrro_el0, &mut input)
            .map_err(KernelError::Memory)?;
        memory
            .check_access(cpu.tpidrro_el0, IPC_BUFFER_SIZE, Access::Write)
            .map_err(KernelError::Memory)?;
        let mut request = parse_request(&input).map_err(KernelError::Ipc)?;
        if request.pid.is_some() {
            request.pid = Some(1);
        } // Owned guest process ID, never the host PID.
        if request.kind == CommandType::Control {
            if request.command_id != 2 {
                return Err(KernelError::UnsupportedControl(request.command_id));
            }
            if !request.data.is_empty() || request.pid.is_some() {
                return Err(KernelError::Ipc(IpcError::UnsupportedLayout));
            }
            let pending = self
                .handles
                .stage(vec![Object::Session(service)])
                .map_err(KernelError::Handle)?;
            let encoded = encode_reply(0, &[], &pending.ids).map_err(KernelError::Ipc)?;
            memory
                .write(cpu.tpidrro_el0, &encoded)
                .map_err(KernelError::Memory)?;
            self.handles.commit(pending);
            self.ipc_records.push(IpcRecord {
                handle,
                kind: request.kind,
                command_id: 2,
                request_bytes: 0,
                reply_bytes: 0,
            });
            cpu.x[0] = 0;
            return Ok(());
        }
        let mut candidate = service.borrow().clone_for_request();
        let reply = candidate.dispatch(&request).map_err(KernelError::Service)?;
        let pending = self
            .handles
            .stage(
                reply
                    .moved_objects
                    .into_iter()
                    .map(Object::session)
                    .collect(),
            )
            .map_err(KernelError::Handle)?;
        let encoded =
            encode_reply(reply.result, &reply.data, &pending.ids).map_err(KernelError::Ipc)?;
        // No callbacks or concurrent mapping changes occur between preflight and
        // publication. Everything fallible is checked before state/handle commit.
        memory
            .write(cpu.tpidrro_el0, &encoded)
            .map_err(KernelError::Memory)?;
        self.handles.commit(pending);
        *service.borrow_mut() = candidate;
        self.ipc_records.push(IpcRecord {
            handle,
            kind: request.kind,
            command_id: request.command_id,
            request_bytes: request.data.len(),
            reply_bytes: reply.data.len(),
        });
        cpu.x[0] = 0;
        Ok(())
    }

    fn create_event(&mut self, cpu: &mut Cpu) -> Result<(), KernelError> {
        let signal = Rc::new(Cell::new(false));
        let pending = self
            .handles
            .stage(vec![
                Object::WritableEvent(Rc::clone(&signal)),
                Object::ReadableEvent(signal),
            ])
            .map_err(KernelError::Handle)?;
        cpu.x[0] = 0;
        cpu.x[1] = u64::from(pending.ids[0]);
        cpu.x[2] = u64::from(pending.ids[1]);
        self.handles.commit(pending);
        Ok(())
    }

    fn wait_poll(&self, cpu: &mut Cpu, memory: &AddressSpace) -> Result<(), KernelError> {
        let count = cpu.x[2] as u32 as i32;
        if count < 0 {
            return Err(KernelError::InvalidWaitCount(count));
        }
        if count > 64 {
            cpu.x[0] = RESULT_TOO_MANY_HANDLES;
            return Ok(());
        }
        let mut input = [0_u8; 256];
        if memory
            .read(cpu.x[1], &mut input[..count as usize * 4])
            .is_err()
        {
            cpu.x[0] = RESULT_INVALID_ADDRESS;
            return Ok(());
        }
        let mut first_signaled = None;
        for (index, bytes) in input[..count as usize * 4].chunks_exact(4).enumerate() {
            let handle = u32::from_le_bytes(bytes.try_into().expect("four bytes"));
            let signaled = match self.handles.get(handle) {
                Ok(Object::ReadableEvent(signal)) => signal.get(),
                Ok(Object::MainThread) => false,
                _ => {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                    return Ok(());
                }
            };
            if signaled && first_signaled.is_none() {
                first_signaled = Some(index);
            }
        }
        if let Some(index) = first_signaled {
            cpu.x[0] = 0;
            cpu.x[1] = index as u64;
        } else if cpu.x[3] == 0 {
            cpu.x[0] = RESULT_TIMEOUT;
        } else {
            return Err(KernelError::UnsupportedWaitTimeout(cpu.x[3] as i64));
        }
        Ok(())
    }

    pub fn dispatch(
        &mut self,
        id: u16,
        cpu: &mut Cpu,
        memory: &mut AddressSpace,
    ) -> Result<KernelEvent, KernelError> {
        if self.exited {
            return Err(KernelError::ProcessAlreadyExited);
        }
        match id {
            SVC_EXIT_PROCESS => {
                self.exited = true;
                self.handles.clear();
                Ok(KernelEvent::ProcessExited)
            }
            SVC_CLOSE_HANDLE => {
                cpu.x[0] = if self.handles.close(cpu.x[0] as u32).is_ok() {
                    0
                } else {
                    RESULT_INVALID_HANDLE
                };
                Ok(KernelEvent::Continue)
            }
            SVC_CREATE_EVENT => {
                self.create_event(cpu)?;
                Ok(KernelEvent::Continue)
            }
            SVC_SIGNAL_EVENT | SVC_CLEAR_EVENT => {
                cpu.x[0] = match self.handles.get(cpu.x[0] as u32) {
                    Ok(Object::WritableEvent(signal)) => {
                        signal.set(id == SVC_SIGNAL_EVENT);
                        0
                    }
                    _ => RESULT_INVALID_HANDLE,
                };
                Ok(KernelEvent::Continue)
            }
            SVC_RESET_SIGNAL => {
                cpu.x[0] = match self.handles.get(cpu.x[0] as u32) {
                    Ok(Object::ReadableEvent(signal)) if signal.replace(false) => 0,
                    Ok(Object::ReadableEvent(_)) => RESULT_INVALID_STATE,
                    _ => RESULT_INVALID_HANDLE,
                };
                Ok(KernelEvent::Continue)
            }
            SVC_WAIT_SYNCHRONIZATION => {
                self.wait_poll(cpu, memory)?;
                Ok(KernelEvent::Continue)
            }
            SVC_CONNECT_TO_NAMED_PORT => {
                self.connect(cpu, memory)?;
                Ok(KernelEvent::Continue)
            }
            SVC_SEND_SYNC_REQUEST => {
                self.send_sync(cpu, memory)?;
                Ok(KernelEvent::Continue)
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
    let mut available = [0_u64; 2];
    for id in [
        SVC_EXIT_PROCESS,
        SVC_OUTPUT_DEBUG_STRING,
        SVC_CLOSE_HANDLE,
        SVC_CONNECT_TO_NAMED_PORT,
        SVC_SEND_SYNC_REQUEST,
        SVC_CREATE_EVENT,
        SVC_SIGNAL_EVENT,
        SVC_CLEAR_EVENT,
        SVC_RESET_SIGNAL,
        SVC_WAIT_SYNCHRONIZATION,
    ] {
        available[id as usize / 64] |= 1_u64 << (id % 64);
    }
    let entries = [
        (1_u32, MAIN_THREAD_HANDLE, 0_u64),
        (7, 0, 0),
        (6, available[0], available[1]),
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
