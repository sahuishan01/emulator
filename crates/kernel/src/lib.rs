//! Original bounded Horizon SVC/handle/IPC and single-core scheduler subset.

mod handles;
mod scheduler;
pub mod service;
mod sync;

use emulator_a64::Cpu;
use emulator_ipc::{encode_reply, parse_request, CommandType, IpcError, IPC_BUFFER_SIZE};
use emulator_memory::{Access, AddressSpace, MemoryError, Permissions};
pub use handles::{HandleError, Object, ProcessIdentity, TransferMemory};
use handles::{HandleTable, ThreadObject};
pub use scheduler::{
    ScheduleOutcome, SchedulerError, SchedulerStats, MAX_THREADS, THREAD_QUANTUM, THREAD_TLS_ARENA,
};
use scheduler::{Scheduler, WaitTarget};
use service::{Service, ServiceContext, ServiceError};
pub use sync::{FutexArbitrator, Waiter};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

pub const SVC_SET_HEAP_SIZE: u16 = 0x01;
pub const SVC_SET_MEMORY_PERMISSION: u16 = 0x02;
pub const SVC_SET_MEMORY_ATTRIBUTE: u16 = 0x03;
pub const SVC_QUERY_MEMORY: u16 = 0x06;
pub const SVC_EXIT_PROCESS: u16 = 0x07;
pub const SVC_CREATE_THREAD: u16 = 0x08;
pub const SVC_START_THREAD: u16 = 0x09;
pub const SVC_EXIT_THREAD: u16 = 0x0a;
pub const SVC_SLEEP_THREAD: u16 = 0x0b;
pub const SVC_SET_THREAD_PRIORITY: u16 = 0x0c;
pub const SVC_GET_THREAD_PRIORITY: u16 = 0x0d;
pub const SVC_GET_THREAD_CORE_MASK: u16 = 0x0e;
pub const SVC_SET_THREAD_CORE_MASK: u16 = 0x0f;
pub const SVC_GET_CURRENT_PROCESSOR_NUMBER: u16 = 0x10;
pub const SVC_SIGNAL_EVENT: u16 = 0x11;
pub const SVC_CLEAR_EVENT: u16 = 0x12;
pub const SVC_MAP_SHARED_MEMORY: u16 = 0x13;
pub const SVC_UNMAP_SHARED_MEMORY: u16 = 0x14;
pub const SVC_CREATE_TRANSFER_MEMORY: u16 = 0x15;
pub const SVC_CLOSE_HANDLE: u16 = 0x16;
pub const SVC_RESET_SIGNAL: u16 = 0x17;
pub const SVC_WAIT_SYNCHRONIZATION: u16 = 0x18;
pub const SVC_CANCEL_SYNCHRONIZATION: u16 = 0x19;
pub const SVC_ARBITRATE_LOCK: u16 = 0x1a;
pub const SVC_ARBITRATE_UNLOCK: u16 = 0x1b;
pub const SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC: u16 = 0x1c;
pub const SVC_SIGNAL_PROCESS_WIDE_KEY: u16 = 0x1d;
pub const SVC_GET_SYSTEM_TICK: u16 = 0x1e;
pub const SVC_CONNECT_TO_NAMED_PORT: u16 = 0x1f;
pub const SVC_SEND_SYNC_REQUEST: u16 = 0x21;
pub const SVC_GET_PROCESS_ID: u16 = 0x24;
pub const SVC_GET_THREAD_ID: u16 = 0x25;
pub const SVC_BREAK: u16 = 0x26;
pub const SVC_OUTPUT_DEBUG_STRING: u16 = 0x27;
pub const SVC_GET_INFO: u16 = 0x29;
pub const SVC_CREATE_EVENT: u16 = 0x45;
pub const CURRENT_PROCESS_HANDLE: u32 = 0xffff_8001;
pub const MAX_TRANSFER_MEMORY_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TRANSFER_MEMORY_OBJECTS: usize = 32;
pub const RESULT_INVALID_SIZE: u64 = 0xca01;
pub const RESULT_OUT_OF_MEMORY: u64 = 0xcc01;
pub const RESULT_INVALID_HANDLE: u64 = 0xe401;
pub const RESULT_INVALID_ADDRESS: u64 = 0xe601;
pub const RESULT_TIMEOUT: u64 = 0xea01;
pub const RESULT_INTERRUPTED: u64 = 0xec01;
pub const RESULT_TOO_MANY_HANDLES: u64 = 0xee01;
pub const RESULT_INVALID_STATE: u64 = 0xfa01;
pub const MAIN_THREAD_HANDLE: u64 = handles::MAIN_THREAD_HANDLE as u64;
pub const DEFAULT_HEAP_BASE: u64 = 0x9000_0000;
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
    InvalidWaitCount(i32),
    Scheduler(SchedulerError),
    InvalidThreadEntry(u64),
    InvalidThreadStack(u64),
    InvalidThreadPriority(i32),
    UnsupportedProcessor(i32),
    InvalidTransferMemory {
        address: u64,
        size: u64,
        permissions: u64,
    },
    TransferMemoryLimit,
    UnsupportedCopyHandle(u32),
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
            Self::InvalidWaitCount(count) => write!(f, "invalid guest wait handle count {count}"),
            Self::Scheduler(error) => write!(f, "guest scheduler error: {error:?}"),
            Self::InvalidThreadEntry(entry) => write!(f, "invalid guest thread entry {entry:#x}"),
            Self::InvalidThreadStack(stack) => write!(f, "invalid guest thread stack {stack:#x}"),
            Self::InvalidThreadPriority(priority) => {
                write!(f, "invalid guest thread priority {priority}")
            }
            Self::InvalidTransferMemory {
                address,
                size,
                permissions,
            } => write!(
                f,
                "unsupported transfer memory: {address:#x}+{size:#x}, permissions {permissions:#x}"
            ),
            Self::TransferMemoryLimit => write!(f, "transfer-memory object limit exceeded"),
            Self::UnsupportedCopyHandle(handle) => {
                write!(f, "unsupported copied handle type: {handle:#x}")
            }
            Self::UnsupportedProcessor(core) => write!(f, "unsupported guest processor {core}"),
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
    pub send_buffers: usize,
    pub receive_buffers: usize,
    pub buffer_bytes: usize,
    pub copy_handles: usize,
}

#[derive(Debug)]
pub struct Kernel {
    debug_records: Vec<Vec<u8>>,
    debug_bytes: usize,
    exited: bool,
    handles: HandleTable,
    ports: BTreeMap<String, Box<dyn Service>>,
    ipc_records: Vec<IpcRecord>,
    scheduler: Scheduler,
    process: ProcessIdentity,
    transfer_quota: Rc<Cell<usize>>,
    heap_base: u64,
    heap_size: usize,
    aslr_base: u64,
    aslr_size: u64,
    program_id: u64,
    sync: FutexArbitrator,
    svc_count: u64,
    last_svc_id: u16,
    last_svc_pc: u64,
    ipc_count: u64,
    last_ipc_cmd: u32,
}

impl Default for Kernel {
    fn default() -> Self {
        let main = ThreadObject::new(0);
        Self {
            debug_records: Vec::new(),
            debug_bytes: 0,
            exited: false,
            handles: HandleTable::new(main.clone()),
            ports: BTreeMap::new(),
            ipc_records: Vec::new(),
            scheduler: Scheduler::new(main),
            process: ProcessIdentity::new(),
            transfer_quota: Rc::new(Cell::new(0)),
            heap_base: DEFAULT_HEAP_BASE,
            heap_size: 0,
            aslr_base: 0x10_0000,
            aslr_size: 0x7f_ff00_0000,
            program_id: 0x0100_0000_0000_1000,
            sync: FutexArbitrator::default(),
            svc_count: 0,
            last_svc_id: 0,
            last_svc_pc: 0,
            ipc_count: 0,
            last_ipc_cmd: 0,
        }
    }
}

impl Kernel {
    pub fn aslr_base(&self) -> u64 {
        self.aslr_base
    }

    pub fn aslr_size(&self) -> u64 {
        self.aslr_size
    }

    pub fn set_aslr_region(&mut self, base: u64, size: u64) {
        self.aslr_base = base;
        self.aslr_size = size;
    }

    pub fn program_id(&self) -> u64 {
        self.program_id
    }

    pub fn set_program_id(&mut self, program_id: u64) {
        self.program_id = program_id;
    }

    pub fn live_transfer_memory_count(&self) -> usize {
        self.transfer_quota.get()
    }
    fn create_transfer_memory(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut AddressSpace,
    ) -> Result<(), KernelError> {
        let address = cpu.x[1];
        let size = cpu.x[2];
        let permissions = u64::from(cpu.x[3] as u32);
        let invalid = || KernelError::InvalidTransferMemory {
            address,
            size,
            permissions,
        };
        let size_usize = usize::try_from(size).map_err(|_| invalid())?;
        let end = address.checked_add(size).ok_or_else(invalid)?;
        if address % 4096 != 0
            || size == 0
            || size % 4096 != 0
            || size_usize > MAX_TRANSFER_MEMORY_BYTES
            || (address < THREAD_TLS_ARENA + MAX_THREADS as u64 * 4096 && THREAD_TLS_ARENA < end)
        {
            return Err(invalid());
        }
        let owner = match permissions {
            0 => emulator_memory::Permissions::NONE,
            1 => emulator_memory::Permissions::READ,
            3 => emulator_memory::Permissions::READ_WRITE,
            _ => return Err(invalid()),
        };
        if self.transfer_quota.get() >= MAX_TRANSFER_MEMORY_OBJECTS {
            return Err(KernelError::TransferMemoryLimit);
        }
        self.handles
            .check_capacity(1)
            .map_err(KernelError::Handle)?;
        let lease = memory
            .lend(address, size_usize, owner)
            .map_err(KernelError::Memory)?;
        let object = Rc::new(TransferMemory::new(
            self.process.clone(),
            lease,
            Rc::clone(&self.transfer_quota),
        ));
        // No handle mutation/callback occurs after capacity preflight. Remaining
        // handle publication cannot return a recoverable error.
        let pending = self
            .handles
            .stage(vec![Object::TransferMemory(object)])
            .expect("preflighted transfer handle slot");
        let handle = pending.ids[0];
        self.handles.commit(pending);
        cpu.x[0] = 0;
        cpu.x[1] = u64::from(handle);
        Ok(())
    }
    fn copied_handles(&self, ids: &[u32]) -> Result<Vec<Object>, KernelError> {
        ids.iter()
            .map(|&id| {
                let object = if id == CURRENT_PROCESS_HANDLE {
                    Object::Process(self.process.clone())
                } else {
                    self.handles.get(id).map_err(KernelError::Handle)?
                };
                match object {
                    Object::Process(_) | Object::TransferMemory(_) => Ok(object),
                    _ => Err(KernelError::UnsupportedCopyHandle(id)),
                }
            })
            .collect()
    }
    pub fn scheduler_stats(&self) -> SchedulerStats {
        self.scheduler.stats()
    }
    pub fn current_thread_id(&self) -> usize {
        self.scheduler.current()
    }
    pub fn current_core(&self) -> u8 {
        self.scheduler.current_core()
    }
    pub fn prepare_execution(&mut self, cpu: &mut Cpu) -> ScheduleOutcome {
        self.scheduler.prepare(cpu)
    }
    pub fn instruction_completed(&mut self) -> Result<(), KernelError> {
        self.scheduler.tick().map_err(KernelError::Scheduler)
    }
    pub fn debug_records(&self) -> &[Vec<u8>] {
        &self.debug_records
    }
    pub fn exited(&self) -> bool {
        self.exited
    }
    pub fn live_handle_count(&self) -> usize {
        self.handles.len()
    }
    /// Explicit diagnostic trace draining for long-running sessions.
    pub fn clear_ipc_trace(&mut self) {
        self.ipc_records.clear();
    }
    pub fn ipc_records(&self) -> &[IpcRecord] {
        &self.ipc_records
    }
    pub fn svc_count(&self) -> u64 {
        self.svc_count
    }
    pub fn last_svc_id(&self) -> u16 {
        self.last_svc_id
    }
    pub fn last_svc_pc(&self) -> u64 {
        self.last_svc_pc
    }
    pub fn ipc_count(&self) -> u64 {
        self.ipc_count
    }
    pub fn last_ipc_cmd(&self) -> u32 {
        self.last_ipc_cmd
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
        self.ipc_records
            .try_reserve(1)
            .map_err(|_| KernelError::IpcTraceLimit)?;
        let mut input = [0; IPC_BUFFER_SIZE];
        memory
            .read(cpu.tpidrro_el0, &mut input)
            .map_err(KernelError::Memory)?;
        memory
            .check_access(cpu.tpidrro_el0, IPC_BUFFER_SIZE, Access::Write)
            .map_err(KernelError::Memory)?;
        let mut request = parse_request(&input).map_err(KernelError::Ipc)?;
        self.ipc_count = self.ipc_count.saturating_add(1);
        self.last_ipc_cmd = request.command_id;
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
                send_buffers: 0,
                receive_buffers: 0,
                buffer_bytes: 0,
                copy_handles: 0,
            });
            cpu.x[0] = 0;
            return Ok(());
        }
        let reply_view = memory
            .pin(
                cpu.tpidrro_el0,
                IPC_BUFFER_SIZE,
                emulator_memory::Permissions::READ_WRITE,
            )
            .map_err(KernelError::Memory)?;
        let copied = self.copied_handles(&request.copy_handles)?;
        let context = ServiceContext::new(memory, &request, self.process.clone(), copied)
            .map_err(KernelError::Service)?;
        context
            .validate_reply(&reply_view)
            .map_err(KernelError::Service)?;
        let mut candidate = service.borrow().clone_for_request();
        let reply = candidate
            .prepare(&request, &context)
            .map_err(KernelError::Service)?;
        if let Some(effect) = &reply.effect {
            let writes = effect.write_ranges();
            if writes.len() > 4096
                || writes.iter().any(|a| {
                    a.length == 0
                        || a.offset.checked_add(a.length).is_none()
                        || reply_view.backing_ranges().any(|b| {
                            a.id == b.id
                                && a.offset < b.offset + b.length
                                && b.offset < a.offset + a.length
                        })
                })
            {
                return Err(KernelError::Service(ServiceError::InvalidEffect));
            }
        }
        let staged_objects: Vec<Object> = reply
            .moved_objects
            .into_iter()
            .map(|s| s.as_handle().unwrap_or_else(|| Object::session(s)))
            .collect();
        let pending = self
            .handles
            .stage(staged_objects)
            .map_err(KernelError::Handle)?;
        let encoded =
            encode_reply(reply.result, &reply.data, &pending.ids).map_err(KernelError::Ipc)?;
        // All validation, encoding and handle staging precede effects. Pins are
        // stable and cannot alias reply writes; the single executor has no
        // callbacks or mapping changes during this infallible publication phase.
        if let Some(mut effect) = reply.effect {
            effect.complete().map_err(KernelError::Service)?;
            effect.commit();
        }
        reply_view
            .write(0, &encoded)
            .expect("prepared IPC reply pin");
        self.handles.commit(pending);
        *service.borrow_mut() = candidate;
        self.ipc_records.push(IpcRecord {
            handle,
            kind: request.kind,
            command_id: request.command_id,
            request_bytes: request.data.len(),
            reply_bytes: reply.data.len(),
            send_buffers: request.send_buffers.len(),
            receive_buffers: request.receive_buffers.len(),
            copy_handles: request.copy_handles.len(),
            buffer_bytes: request
                .send_buffers
                .iter()
                .chain(&request.receive_buffers)
                .map(|d| d.size as usize)
                .sum(),
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

    fn create_thread(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut AddressSpace,
    ) -> Result<(), KernelError> {
        let id = self
            .scheduler
            .next_thread()
            .map_err(KernelError::Scheduler)?;
        let entry = cpu.x[1];
        if entry % 4 != 0 || memory.check_access(entry, 4, Access::Execute).is_err() {
            return Err(KernelError::InvalidThreadEntry(entry));
        }
        let stack = cpu.x[3];
        if stack < 16
            || stack % 16 != 0
            || memory.check_access(stack - 16, 16, Access::Write).is_err()
        {
            return Err(KernelError::InvalidThreadStack(stack));
        }
        let priority = cpu.x[4] as u32 as i32;
        if !(0..64).contains(&priority) {
            return Err(KernelError::InvalidThreadPriority(priority));
        }
        let core = cpu.x[5] as u32 as i32;
        if core != 0 && core != -2 {
            return Err(KernelError::UnsupportedProcessor(core));
        }
        let object = ThreadObject::new(id);
        let pending = self
            .handles
            .stage(vec![Object::Thread(object.clone())])
            .map_err(KernelError::Handle)?;
        let tls_page = THREAD_TLS_ARENA + id as u64 * 4096;
        let mut context = Cpu::new(entry, stack);
        context.x[0] = cpu.x[2];
        context.tpidrro_el0 = tls_page + 0x200;
        // No fallible publication follows the transactional guest mapping.
        memory
            .map(
                tls_page,
                vec![0; 4096],
                emulator_memory::Permissions::READ_WRITE,
            )
            .map_err(KernelError::Memory)?;
        self.scheduler.add(object, context, priority as u8, core);
        let _ = memory.set_state(tls_page, 4096, 0x0c);
        cpu.x[0] = 0;
        cpu.x[1] = u64::from(pending.ids[0]);
        self.handles.commit(pending);
        Ok(())
    }

    fn wait_poll(&mut self, cpu: &mut Cpu, memory: &AddressSpace) -> Result<(), KernelError> {
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
        let mut targets = Vec::with_capacity(count as usize);
        for (index, bytes) in input[..count as usize * 4].chunks_exact(4).enumerate() {
            let handle = u32::from_le_bytes(bytes.try_into().expect("four bytes"));
            let target = match self.handles.get(handle) {
                Ok(Object::ReadableEvent(signal)) => WaitTarget(signal),
                Ok(Object::Thread(object)) => WaitTarget(object.terminated),
                _ => {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                    return Ok(());
                }
            };
            if target.signaled() && first_signaled.is_none() {
                first_signaled = Some(index);
            }
            targets.push(target);
        }
        if let Some(index) = first_signaled {
            cpu.x[0] = 0;
            cpu.x[1] = index as u64;
        } else if cpu.x[3] == 0 {
            cpu.x[0] = RESULT_TIMEOUT;
        } else if self.scheduler.take_cancellation() {
            cpu.x[0] = RESULT_INTERRUPTED;
        } else {
            self.scheduler
                .wait(targets, cpu.x[3] as i64)
                .map_err(KernelError::Scheduler)?;
        }
        Ok(())
    }

    fn query_memory(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut AddressSpace,
    ) -> Result<(), KernelError> {
        let out_meminfo_addr = cpu.x[0];
        let in_pageinfo_addr = cpu.x[1];
        let query_addr = cpu.x[2];
        if out_meminfo_addr == 0 || out_meminfo_addr % 4 != 0 {
            cpu.x[0] = RESULT_INVALID_ADDRESS;
            return Ok(());
        }
        if query_addr >= (1_u64 << 39) {
            cpu.x[0] = RESULT_INVALID_ADDRESS;
            return Ok(());
        }
        let desc = memory.query_memory(query_addr);
        let mut bytes = [0u8; 40];
        bytes[0..8].copy_from_slice(&desc.base_address.to_le_bytes());
        bytes[8..16].copy_from_slice(&desc.size.to_le_bytes());
        bytes[16..20].copy_from_slice(&desc.state.to_le_bytes());
        bytes[20..24].copy_from_slice(&desc.attribute.to_le_bytes());
        bytes[24..28].copy_from_slice(&desc.permission.to_le_bytes());
        bytes[28..32].copy_from_slice(&desc.ipc_count.to_le_bytes());
        bytes[32..36].copy_from_slice(&desc.device_count.to_le_bytes());
        bytes[36..40].copy_from_slice(&0_u32.to_le_bytes());

        if memory.write(out_meminfo_addr, &bytes).is_err() {
            cpu.x[0] = RESULT_INVALID_ADDRESS;
            return Ok(());
        }
        if in_pageinfo_addr != 0 && in_pageinfo_addr % 4 == 0 {
            let _ = memory.write_u32(in_pageinfo_addr, 0);
        }
        cpu.x[0] = 0;
        cpu.x[1] = 0;
        Ok(())
    }

    fn set_heap_size(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut AddressSpace,
    ) -> Result<(), KernelError> {
        let out_ptr = cpu.x[0];
        let size = cpu.x[1] as usize;
        if size % 4096 != 0 {
            cpu.x[0] = RESULT_INVALID_SIZE;
            return Ok(());
        }
        let heap_base = self.heap_base;
        let current_size = self.heap_size;
        if size == current_size {
            if out_ptr != 0 && out_ptr % 4 == 0 {
                let _ = memory.write_u64(out_ptr, heap_base);
            }
            cpu.x[0] = 0;
            cpu.x[1] = heap_base;
            return Ok(());
        }
        if size > current_size {
            let additional = size - current_size;
            let start = heap_base + current_size as u64;
            if memory
                .map(
                    start,
                    vec![0; additional],
                    emulator_memory::Permissions::READ_WRITE,
                )
                .is_err()
            {
                cpu.x[0] = RESULT_OUT_OF_MEMORY;
                return Ok(());
            }
            let _ = memory.set_state(start, additional, 0x03);
        } else {
            let reduction = current_size - size;
            let start = heap_base + size as u64;
            if memory.unmap(start, reduction).is_err() {
                cpu.x[0] = RESULT_INVALID_STATE;
                return Ok(());
            }
        }
        self.heap_size = size;
        if out_ptr != 0 && out_ptr % 4 == 0 {
            let _ = memory.write_u64(out_ptr, heap_base);
        }
        cpu.x[0] = 0;
        cpu.x[1] = heap_base;
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
        self.svc_count = self.svc_count.saturating_add(1);
        self.last_svc_id = id;
        self.last_svc_pc = cpu.pc;
        match id {
            SVC_SET_HEAP_SIZE => {
                self.set_heap_size(cpu, memory)?;
                Ok(KernelEvent::Continue)
            }
            SVC_SET_MEMORY_PERMISSION => {
                let addr = cpu.x[0];
                let size = cpu.x[1] as usize;
                let perm = cpu.x[2] as u32;
                let p = match perm {
                    0 => Permissions::NONE,
                    1 => Permissions::READ,
                    2 => Permissions::WRITE,
                    3 => Permissions::READ_WRITE,
                    5 => Permissions::READ_EXECUTE,
                    7 => Permissions::READ_WRITE_EXECUTE,
                    _ => Permissions::NONE,
                };
                if memory.protect(addr, size, p).is_ok() {
                    cpu.x[0] = 0;
                } else {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_SET_MEMORY_ATTRIBUTE => {
                cpu.x[0] = 0;
                Ok(KernelEvent::Continue)
            }
            SVC_QUERY_MEMORY => {
                self.query_memory(cpu, memory)?;
                Ok(KernelEvent::Continue)
            }
            SVC_EXIT_PROCESS => {
                self.exited = true;
                self.scheduler.terminate_all();
                self.handles.clear();
                Ok(KernelEvent::ProcessExited)
            }
            SVC_CREATE_THREAD => {
                self.create_thread(cpu, memory)?;
                Ok(KernelEvent::Continue)
            }
            SVC_START_THREAD => {
                cpu.x[0] = match self.handles.get(cpu.x[0] as u32) {
                    Ok(Object::Thread(object)) => {
                        if self.scheduler.start(object.id) {
                            0
                        } else {
                            RESULT_INVALID_STATE
                        }
                    }
                    _ => RESULT_INVALID_HANDLE,
                };
                Ok(KernelEvent::Continue)
            }
            SVC_EXIT_THREAD => {
                if self.scheduler.exit_current() {
                    self.exited = true;
                    self.scheduler.terminate_all();
                    self.handles.clear();
                    Ok(KernelEvent::ProcessExited)
                } else {
                    Ok(KernelEvent::Continue)
                }
            }
            SVC_SLEEP_THREAD => {
                self.scheduler
                    .sleep(cpu.x[0] as i64)
                    .map_err(KernelError::Scheduler)?;
                Ok(KernelEvent::Continue)
            }
            SVC_SET_THREAD_PRIORITY => {
                let handle = cpu.x[0] as u32;
                let priority = cpu.x[1] as u8;
                let thread_id = if handle == CURRENT_PROCESS_HANDLE || handle == MAIN_THREAD_HANDLE as u32 {
                    self.scheduler.current()
                } else if let Ok(Object::Thread(t)) = self.handles.get(handle) {
                    t.id
                } else {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                    return Ok(KernelEvent::Continue);
                };
                if self.scheduler.set_thread_priority(thread_id, priority) {
                    cpu.x[0] = 0;
                } else {
                    cpu.x[0] = RESULT_INVALID_STATE;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_GET_THREAD_PRIORITY => {
                let handle = cpu.x[1] as u32;
                let thread_id = if handle == CURRENT_PROCESS_HANDLE || handle == MAIN_THREAD_HANDLE as u32 {
                    self.scheduler.current()
                } else if let Ok(Object::Thread(t)) = self.handles.get(handle) {
                    t.id
                } else {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                    return Ok(KernelEvent::Continue);
                };
                if let Some(priority) = self.scheduler.get_thread_priority(thread_id) {
                    cpu.x[0] = 0;
                    cpu.x[1] = priority as u64;
                } else {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_GET_THREAD_CORE_MASK => {
                let handle = cpu.x[2] as u32;
                let thread_id = if handle == CURRENT_PROCESS_HANDLE || handle == MAIN_THREAD_HANDLE as u32 {
                    self.scheduler.current()
                } else if let Ok(Object::Thread(t)) = self.handles.get(handle) {
                    t.id
                } else {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                    return Ok(KernelEvent::Continue);
                };
                if let Some((ideal, mask)) = self.scheduler.get_thread_core_mask(thread_id) {
                    cpu.x[0] = 0;
                    cpu.x[1] = ideal as u64;
                    cpu.x[2] = mask;
                } else {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_SET_THREAD_CORE_MASK => {
                let handle = cpu.x[0] as u32;
                let ideal_core = cpu.x[1] as i32;
                let mask = cpu.x[2];
                let thread_id = if handle == CURRENT_PROCESS_HANDLE || handle == MAIN_THREAD_HANDLE as u32 {
                    self.scheduler.current()
                } else if let Ok(Object::Thread(t)) = self.handles.get(handle) {
                    t.id
                } else {
                    cpu.x[0] = RESULT_INVALID_HANDLE;
                    return Ok(KernelEvent::Continue);
                };
                if self.scheduler.set_thread_core_mask(thread_id, ideal_core, mask) {
                    cpu.x[0] = 0;
                } else {
                    cpu.x[0] = RESULT_INVALID_STATE;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_GET_CURRENT_PROCESSOR_NUMBER => {
                cpu.x[0] = self.scheduler.current_core() as u64;
                Ok(KernelEvent::Continue)
            }
            SVC_MAP_SHARED_MEMORY => {
                let handle = cpu.x[0] as u32;
                let addr = cpu.x[1];
                let size = cpu.x[2] as usize;
                let perms = cpu.x[3];
                if addr % 4096 != 0 || size == 0 || size % 4096 != 0 {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                let permission = match perms {
                    1 => Permissions::READ,
                    3 => Permissions::READ_WRITE,
                    _ => Permissions::READ,
                };
                match self.handles.get(handle) {
                    Ok(Object::SharedMemory(buf)) => {
                        let buffer = buf.borrow();
                        let data = if buffer.len() >= size {
                            buffer[..size].to_vec()
                        } else {
                            let mut d = buffer.clone();
                            d.resize(size, 0);
                            d
                        };
                        if memory.map(addr, data, permission).is_ok() {
                            let _ = memory.set_state(addr, size, 0x0D);
                            cpu.x[0] = 0;
                        } else {
                            cpu.x[0] = RESULT_OUT_OF_MEMORY;
                        }
                    }
                    _ => {
                        cpu.x[0] = RESULT_INVALID_HANDLE;
                    }
                }
                Ok(KernelEvent::Continue)
            }
            SVC_UNMAP_SHARED_MEMORY => {
                let _handle = cpu.x[0] as u32;
                let addr = cpu.x[1];
                let size = cpu.x[2] as usize;
                if addr % 4096 != 0 || size == 0 || size % 4096 != 0 {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                } else {
                    let _ = memory.unmap(addr, size);
                    cpu.x[0] = 0;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_ARBITRATE_LOCK => {
                let owner_tag = cpu.x[0] as u32;
                let mutex_addr = cpu.x[1];
                let tag = cpu.x[2] as u32;
                if mutex_addr & 3 != 0 {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                let current_val = match memory.read_u32(mutex_addr) {
                    Ok(val) => val,
                    Err(_) => {
                        cpu.x[0] = RESULT_INVALID_ADDRESS;
                        return Ok(KernelEvent::Continue);
                    }
                };
                let my_tag = if tag != 0 { tag } else { owner_tag };
                if current_val == 0 {
                    let _ = memory.write_u32(mutex_addr, my_tag);
                    cpu.x[0] = 0;
                } else if current_val == tag
                    || current_val == owner_tag
                    || current_val == (owner_tag | 0x4000_0000)
                {
                    let _ = memory.write_u32(mutex_addr, current_val | 0x4000_0000);
                    let (signal, _) = self.sync.lock_mutex(self.scheduler.current(), my_tag, mutex_addr);
                    if self.scheduler.wait(vec![WaitTarget(signal)], -1).is_err() {
                        cpu.x[0] = RESULT_INVALID_STATE;
                    }
                } else {
                    cpu.x[0] = 0;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_ARBITRATE_UNLOCK => {
                let mutex_addr = cpu.x[0];
                if mutex_addr & 3 != 0 {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                if let Some(waiter) = self.sync.unlock_mutex(mutex_addr) {
                    let tag = if waiter.tag != 0 {
                        waiter.tag
                    } else {
                        waiter.thread_index as u32 + 1
                    };
                    let new_tag = if self.sync.has_mutex_waiters(mutex_addr) {
                        tag | 0x4000_0000
                    } else {
                        tag
                    };
                    let _ = memory.write_u32(mutex_addr, new_tag);
                } else {
                    let _ = memory.write_u32(mutex_addr, 0);
                }
                cpu.x[0] = 0;
                Ok(KernelEvent::Continue)
            }
            SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC => {
                let mutex_addr = cpu.x[0];
                let key_addr = cpu.x[1];
                let tag = cpu.x[2] as u32;
                let timeout_ns = cpu.x[3] as i64;
                if (key_addr & 3 != 0) || (mutex_addr & 3 != 0) {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                if memory.read_u32(mutex_addr).is_err() {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                if let Some(waiter) = self.sync.unlock_mutex(mutex_addr) {
                    let w_tag = if waiter.tag != 0 {
                        waiter.tag
                    } else {
                        waiter.thread_index as u32 + 1
                    };
                    let new_tag = if self.sync.has_mutex_waiters(mutex_addr) {
                        w_tag | 0x4000_0000
                    } else {
                        w_tag
                    };
                    let _ = memory.write_u32(mutex_addr, new_tag);
                } else {
                    let _ = memory.write_u32(mutex_addr, 0);
                }
                if timeout_ns == 0 {
                    cpu.x[0] = RESULT_TIMEOUT;
                    return Ok(KernelEvent::Continue);
                }
                let (signal, _) = self.sync.wait_condition(self.scheduler.current(), tag, key_addr, mutex_addr);
                if self.scheduler.wait(vec![WaitTarget(signal)], timeout_ns).is_err() {
                    cpu.x[0] = RESULT_INVALID_STATE;
                } else {
                    cpu.x[0] = 0;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_SIGNAL_PROCESS_WIDE_KEY => {
                let key_addr = cpu.x[0];
                let count = cpu.x[1] as i32;
                if key_addr & 3 != 0 {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                let woken = self.sync.signal_condition(key_addr, count);
                for waiter in woken {
                    let m_addr = waiter.mutex_addr;
                    let cur_mutex = memory.read_u32(m_addr).unwrap_or(0);
                    if cur_mutex == 0 {
                        let tag = if waiter.tag != 0 {
                            waiter.tag
                        } else {
                            waiter.thread_index as u32 + 1
                        };
                        let _ = memory.write_u32(m_addr, tag);
                        waiter.signal.set(true);
                    } else {
                        let _ = memory.write_u32(m_addr, cur_mutex | 0x4000_0000);
                        self.sync.requeue_to_mutex(waiter);
                    }
                }
                if self.sync.condvar_waiter_count(key_addr) == 0 {
                    let _ = memory.write_u32(key_addr, 0);
                }
                cpu.x[0] = 0;
                Ok(KernelEvent::Continue)
            }
            SVC_GET_SYSTEM_TICK => {
                let ns = self.scheduler.stats().virtual_time_ns;
                cpu.x[0] = (ns.saturating_mul(192) / 10000).max(1);
                Ok(KernelEvent::Continue)
            }
            SVC_GET_PROCESS_ID => {
                let out_ptr = cpu.x[0];
                let _process_handle = cpu.x[1] as u32;
                if out_ptr != 0 && out_ptr % 4 == 0 {
                    let _ = memory.write_u64(out_ptr, 1);
                }
                cpu.x[0] = 0;
                cpu.x[1] = 1;
                Ok(KernelEvent::Continue)
            }
            SVC_GET_THREAD_ID => {
                let out_ptr = cpu.x[0];
                let thread_handle = cpu.x[1] as u32;
                let thread_id = if thread_handle == 0xffff_8000
                    || thread_handle == CURRENT_PROCESS_HANDLE
                    || thread_handle == MAIN_THREAD_HANDLE as u32
                {
                    (self.scheduler.current() as u64) + 1
                } else {
                    match self.handles.get(thread_handle) {
                        Ok(Object::Thread(obj)) => (obj.id as u64) + 1,
                        _ => 1,
                    }
                };
                if out_ptr != 0 && out_ptr % 4 == 0 {
                    let _ = memory.write_u64(out_ptr, thread_id);
                }
                cpu.x[0] = 0;
                cpu.x[1] = thread_id;
                Ok(KernelEvent::Continue)
            }
            SVC_GET_INFO => {
                let out_ptr = cpu.x[0];
                let info_type = cpu.x[1] as u32;
                let _handle = cpu.x[2] as u32;
                let _subtype = cpu.x[3];
                let val: u64 = match info_type {
                    0 => 7, // InfoType_CoreMask (cores 0, 1, 2)
                    1 => !0u64, // InfoType_PriorityMask (all priorities)
                    2 => 0x20_0000_0000, // InfoType_AliasRegionAddress
                    3 => 0x10_0000_0000, // InfoType_AliasRegionSize
                    4 => self.heap_base, // InfoType_HeapRegionAddress
                    5 => 0x4000_0000, // InfoType_HeapRegionSize (1 GB)
                    6 => 0x1_0000_0000, // InfoType_TotalMemorySize (4 GB)
                    7 => 0x2000_0000, // InfoType_UsedMemorySize (512 MB)
                    8 => 0, // InfoType_DebuggerAttached
                    9 => 0, // InfoType_ResourceLimit
                    10 => 0, // InfoType_IdleTickCount
                    11 => 0x0123_4567_89ab_cdef, // InfoType_RandomEntropy
                    12 => self.aslr_base, // InfoType_AslrRegionAddress
                    13 => self.aslr_size, // InfoType_AslrRegionSize
                    14 => 0x70_0000_0000, // InfoType_StackRegionAddress
                    15 => 0x1000_0000, // InfoType_StackRegionSize
                    16 => 0, // InfoType_SystemResourceSizeTotal
                    17 => 0, // InfoType_SystemResourceSizeUsed
                    18 => self.program_id, // InfoType_ProgramId
                    19 => 0, // InfoType_InitialProcessIdRange
                    20 => 0, // InfoType_UserExceptionContextAddress
                    21 => 0x1_0000_0000, // InfoType_TotalNonSystemMemorySize
                    22 => 0x2000_0000, // InfoType_UsedNonSystemMemorySize
                    23 => 1, // InfoType_IsApplication (true)
                    24 => 128, // InfoType_FreeThreadCount
                    25 => 0, // InfoType_ThreadTickCount
                    26 => 1, // InfoType_IsSvcPermitted
                    _ => 0,
                };
                if out_ptr != 0 && out_ptr % 4 == 0 {
                    let _ = memory.write_u64(out_ptr, val);
                }
                cpu.x[0] = 0;
                cpu.x[1] = val;
                Ok(KernelEvent::Continue)
            }
            SVC_BREAK | 0x2c => {
                let reason = cpu.x[0] as u32;
                let info1 = cpu.x[1];
                let info2 = cpu.x[2];
                let pc = cpu.pc;
                let msg = format!("guest svcBreak: reason={reason:#x}, info1={info1:#x}, info2={info2:#x} at pc={pc:#x}\n");
                self.debug_records.push(msg.into_bytes());
                cpu.x[0] = 0;
                Ok(KernelEvent::Continue)
            }
            SVC_CANCEL_SYNCHRONIZATION => {
                cpu.x[0] = match self.handles.get(cpu.x[0] as u32) {
                    Ok(Object::Thread(object)) => {
                        self.scheduler.cancel_wait(object.id);
                        0
                    }
                    _ => RESULT_INVALID_HANDLE,
                };
                Ok(KernelEvent::Continue)
            }
            SVC_CREATE_TRANSFER_MEMORY => {
                self.create_transfer_memory(cpu, memory)?;
                Ok(KernelEvent::Continue)
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
                    Ok(Object::ReadableEvent(signal)) if id == SVC_CLEAR_EVENT => {
                        signal.set(false);
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
        SVC_SET_HEAP_SIZE,
        SVC_SET_MEMORY_PERMISSION,
        SVC_SET_MEMORY_ATTRIBUTE,
        SVC_QUERY_MEMORY,
        SVC_EXIT_PROCESS,
        SVC_CREATE_THREAD,
        SVC_START_THREAD,
        SVC_EXIT_THREAD,
        SVC_SLEEP_THREAD,
        SVC_SET_THREAD_PRIORITY,
        SVC_GET_THREAD_PRIORITY,
        SVC_GET_THREAD_CORE_MASK,
        SVC_SET_THREAD_CORE_MASK,
        SVC_GET_CURRENT_PROCESSOR_NUMBER,
        SVC_MAP_SHARED_MEMORY,
        SVC_UNMAP_SHARED_MEMORY,
        SVC_OUTPUT_DEBUG_STRING,
        SVC_CLOSE_HANDLE,
        SVC_CREATE_TRANSFER_MEMORY,
        SVC_CONNECT_TO_NAMED_PORT,
        SVC_SEND_SYNC_REQUEST,
        SVC_GET_SYSTEM_TICK,
        SVC_GET_PROCESS_ID,
        SVC_GET_THREAD_ID,
        SVC_GET_INFO,
        SVC_BREAK,
        SVC_CREATE_EVENT,
        SVC_SIGNAL_EVENT,
        SVC_CLEAR_EVENT,
        SVC_RESET_SIGNAL,
        SVC_WAIT_SYNCHRONIZATION,
        SVC_CANCEL_SYNCHRONIZATION,
        SVC_ARBITRATE_LOCK,
        SVC_ARBITRATE_UNLOCK,
        SVC_WAIT_PROCESS_WIDE_KEY_ATOMIC,
        SVC_SIGNAL_PROCESS_WIDE_KEY,
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
