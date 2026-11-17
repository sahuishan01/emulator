//! Original bounded Horizon SVC/handle/IPC and single-core scheduler subset.

mod handles;
mod scheduler;
pub mod service;
mod sync;

use emulator_a64::Cpu;
use emulator_ipc::{encode_reply, CommandType, IpcError, IPC_BUFFER_SIZE};
use emulator_memory::{Access, AddressSpace, MemoryError, Permissions};
pub use handles::{HandleError, Object, ProcessIdentity, TransferMemory};
use handles::{HandleTable, ThreadObject};
pub use scheduler::{ThreadSummary, 
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
pub const SVC_MAP_MEMORY: u16 = 0x04;
pub const SVC_UNMAP_MEMORY: u16 = 0x05;
pub const SVC_QUERY_MEMORY: u16 = 0x06;
pub const SVC_EXIT_PROCESS: u16 = 0x07;
pub const SVC_CREATE_THREAD: u16 = 0x08;
pub const SVC_START_THREAD: u16 = 0x09;
pub const SVC_EXIT_THREAD: u16 = 0x0a;
/// Process user-exception context slot (svcGetInfo 20): first 0x200 bytes of
/// the main thread's TLS page.
pub const USER_EXCEPTION_CONTEXT: u64 = THREAD_TLS_ARENA;
pub const SVC_SLEEP_THREAD: u16 = 0x0b;
// Horizon numbering: 0x0C reads the priority, 0x0D writes it.
pub const SVC_GET_THREAD_PRIORITY: u16 = 0x0c;
pub const SVC_SET_THREAD_PRIORITY: u16 = 0x0d;
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
pub const SVC_MAP_PHYSICAL_MEMORY: u16 = 0x2c;
pub const SVC_UNMAP_PHYSICAL_MEMORY: u16 = 0x2d;
pub const SVC_WAIT_FOR_ADDRESS: u16 = 0x34;
pub const SVC_SIGNAL_TO_ADDRESS: u16 = 0x35;
pub const SVC_CREATE_EVENT: u16 = 0x45;
pub const CURRENT_PROCESS_HANDLE: u32 = 0xffff_8001;
/// Upper bound for one transfer memory. Leases share the guest's backing (no
/// copy), so this only rejects absurd sizes; games lend tens of MiB to services.
pub const MAX_TRANSFER_MEMORY_BYTES: usize = 1024 * 1024 * 1024;
pub const MAX_TRANSFER_MEMORY_OBJECTS: usize = 32;
pub const RESULT_INVALID_SIZE: u64 = 0xca01;
/// The addressed range is not in the state the call requires.
pub const RESULT_INVALID_CURRENT_MEMORY: u64 = 0xd401;
/// Host backing for the guest could not be allocated (kernel OutOfMemory).
pub const RESULT_OUT_OF_MEMORY: u64 = 0xd001;
pub const RESULT_INVALID_HANDLE: u64 = 0xe401;
/// Misaligned or out-of-region guest address argument.
pub const RESULT_INVALID_ADDRESS: u64 = 0xcc01;
/// A user buffer the kernel must read or write is not accessible.
pub const RESULT_INVALID_POINTER: u64 = 0xe601;
/// The process resource limit (physical memory) would be exceeded.
pub const RESULT_LIMIT_REACHED: u64 = 0x10801;
pub const RESULT_TIMEOUT: u64 = 0xea01;
pub const RESULT_INTERRUPTED: u64 = 0xec01;
pub const RESULT_TOO_MANY_HANDLES: u64 = 0xee01;
pub const RESULT_SESSION_CLOSED: u64 = 0xf601;
pub const RESULT_INVALID_STATE: u64 = 0xfa01;
pub const MAIN_THREAD_HANDLE: u64 = handles::MAIN_THREAD_HANDLE as u64;
pub const DEFAULT_HEAP_BASE: u64 = 0x9000_0000;
/// Heap region size of a 39-bit application address space.
pub const HEAP_REGION_SIZE: u64 = 0x1_8000_0000;
/// Application memory pool reported as total (non-system) memory.
pub const APPLICATION_POOL_SIZE: u64 = 0xcd50_0000;
/// Kernel result for an unknown enumeration value (module 1, desc 120).
pub const RESULT_INVALID_ENUM_VALUE: u64 = 0xf001;
/// Kernel result for an address outside the required region (desc 110).
pub const RESULT_INVALID_MEMORY_REGION: u64 = 0xdc01;
/// Alias region of the 39-bit application layout (InfoType 2/3).
pub const ALIAS_REGION_BASE: u64 = 0x20_0000_0000;
pub const ALIAS_REGION_SIZE: u64 = 0x10_0000_0000;
/// Memory state of pages backed by svcMapPhysicalMemory (Normal).
pub const PHYSICAL_MEMORY_STATE: u32 = 0x05;
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
    /// Domain message naming an object the session does not own, or passing
    /// input objects (unsupported).
    InvalidDomainObject(u32),
    IpcTraceLimit,
    InvalidWaitCount(i32),
    Scheduler(SchedulerError),
    InvalidThreadEntry(u64),
    InvalidThreadStack(u64),
    InvalidThreadPriority(i32),
    UnsupportedProcessor(i32),
    /// The guest aborted itself with svcBreak.
    GuestBreak { reason: u32, info1: u64, info2: u64 },
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
            Self::InvalidDomainObject(id) => write!(f, "invalid CMIF domain object {id}"),
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
            Self::GuestBreak {
                reason,
                info1,
                info2,
            } => write!(
                f,
                "guest aborted with svcBreak: reason={reason:#x} info1={info1:#x} info2={info2:#x}"
            ),
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

/// Number of IPC messages kept for post-mortem diagnostics.
pub const IPC_TRACE_CAPACITY: usize = 48;

/// One IPC message as far as it was understood. `result` is `None` when the
/// kernel or service rejected the message instead of replying.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IpcTrace {
    pub sequence: u64,
    pub handle: u32,
    pub service: &'static str,
    pub control: bool,
    pub tipc: bool,
    pub domain_object: Option<u32>,
    pub command_id: Option<u32>,
    pub request_bytes: usize,
    pub buffers: [usize; 4],
    pub copy_handles: usize,
    pub result: Option<u32>,
    /// First words of the message as sent, for diagnosing rejections.
    pub words: [u32; 20],
}

/// Number of supervisor calls kept for post-mortem diagnostics.
/// Per-session pointer buffer size reported by QueryPointerBufferSize.
pub const POINTER_BUFFER_SIZE: u16 = 0x8000;

pub const SVC_TRACE_CAPACITY: usize = 128;

/// One dispatched supervisor call: register arguments on entry and, when the
/// handler completed, `x0..x2` on return. `results` is `None` for a call the
/// kernel rejected as unsupported or faulting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvcRecord {
    pub sequence: u64,
    pub id: u16,
    pub pc: u64,
    pub arguments: [u64; 4],
    pub results: Option<[u64; 3]>,
}

/// Memory-management supervisor calls kept for post-mortem accounting
/// diagnostics (heap, physical, shared/transfer maps and memory GetInfo).
pub const MEMORY_SVC_TRACE_CAPACITY: usize = 512;

/// A memory-management supervisor call plus guest-mapped bytes afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemorySvcRecord {
    pub call: SvcRecord,
    pub mapped_after: u64,
}

fn is_memory_svc(id: u16, info_type: u64) -> bool {
    matches!(id, 0x01 | 0x04 | 0x05 | 0x13 | 0x14 | 0x15 | 0x2c | 0x2d | 0x51 | 0x52)
        || (id == 0x29 && matches!(info_type, 6 | 7 | 16 | 17 | 21 | 22))
}

type SessionObject = Rc<std::cell::RefCell<Box<dyn Service>>>;

/// Object table of one session converted with ConvertCurrentObjectToDomain.
/// The session is identified by a weak reference, so a dropped session can
/// never alias a later allocation.
#[derive(Debug)]
struct Domain {
    session: std::rc::Weak<std::cell::RefCell<Box<dyn Service>>>,
    objects: BTreeMap<u32, SessionObject>,
    next_id: u32,
}
pub const MAX_DOMAIN_OBJECTS: usize = 4096;
pub const MAX_IPC_RECORDS: usize = 4096;
pub const MAX_MEMORY_ALIASES: usize = 1024;

/// Page-aligned, non-empty, non-overlapping alias request size.
fn alias_size(destination: u64, source: u64, size: u64) -> Option<usize> {
    let size = usize::try_from(size).ok()?;
    let aligned = destination % 4096 == 0 && source % 4096 == 0 && size % 4096 == 0;
    let destination_end = destination.checked_add(size as u64)?;
    let source_end = source.checked_add(size as u64)?;
    (aligned && size != 0 && (destination_end <= source || source_end <= destination))
        .then_some(size)
}

#[derive(Debug)]
pub struct Kernel {
    domains: Vec<Domain>,
    /// Live svcMapMemory aliases as (destination, source, size).
    memory_aliases: Vec<(u64, u64, usize)>,
    debug_records: Vec<Vec<u8>>,
    debug_bytes: usize,
    exited: bool,
    handles: HandleTable,
    /// Diagnostic only: which service command (or SVC) produced each handle.
    handle_origins: BTreeMap<u32, String>,
    ports: BTreeMap<String, Box<dyn Service>>,
    ipc_records: Vec<IpcRecord>,
    ipc_trace: std::collections::VecDeque<IpcTrace>,
    scheduler: Scheduler,
    process: ProcessIdentity,
    transfer_quota: Rc<Cell<usize>>,
    heap_base: u64,
    heap_size: usize,
    aslr_base: u64,
    aslr_size: u64,
    program_id: u64,
    /// NPDM system resource size; nonzero enables svcMapPhysicalMemory.
    system_resource_size: u64,
    /// Bytes currently backed through svcMapPhysicalMemory.
    physical_mapped: u64,
    /// Process image (code, data, bss) plus main thread stack, charged to
    /// the resource limit at process creation.
    fixed_charge: u64,
    sync: FutexArbitrator,
    svc_count: u64,
    svc_records: std::collections::VecDeque<SvcRecord>,
    memory_svc_records: std::collections::VecDeque<MemorySvcRecord>,
    last_svc_id: u16,
    last_svc_pc: u64,
    ipc_count: u64,
    last_ipc_cmd: u32,
    last_ipc_service: &'static str,
    ipc_histogram: BTreeMap<(&'static str, u32), u64>,
}

impl Default for Kernel {
    fn default() -> Self {
        let main = ThreadObject::new(0);
        Self {
            domains: Vec::new(),
            memory_aliases: Vec::new(),
            debug_records: Vec::new(),
            debug_bytes: 0,
            exited: false,
            handles: HandleTable::new(main.clone()),
            handle_origins: BTreeMap::new(),
            ports: BTreeMap::new(),
            ipc_records: Vec::new(),
            ipc_trace: std::collections::VecDeque::with_capacity(IPC_TRACE_CAPACITY),
            scheduler: Scheduler::new(main),
            process: ProcessIdentity::new(),
            transfer_quota: Rc::new(Cell::new(0)),
            heap_base: DEFAULT_HEAP_BASE,
            heap_size: 0,
            aslr_base: 0x10_0000,
            aslr_size: 0x7f_ff00_0000,
            program_id: 0x0100_0000_0000_1000,
            system_resource_size: 0,
            physical_mapped: 0,
            fixed_charge: 0,
            sync: FutexArbitrator::default(),
            svc_count: 0,
            svc_records: std::collections::VecDeque::with_capacity(SVC_TRACE_CAPACITY),
            memory_svc_records: std::collections::VecDeque::new(),
            last_svc_id: 0,
            last_svc_pc: 0,
            ipc_count: 0,
            last_ipc_cmd: 0,
            last_ipc_service: "",
            ipc_histogram: BTreeMap::new(),
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

    pub fn set_system_resource_size(&mut self, size: u64) {
        self.system_resource_size = size;
    }

    pub fn physical_mapped(&self) -> u64 {
        self.physical_mapped
    }

    /// Sets the creation-time charge: loaded image bytes plus main stack.
    pub fn set_fixed_memory_charge(&mut self, bytes: u64) {
        self.fixed_charge = bytes;
    }

    /// Physical memory charged to the process outside its system resource:
    /// image, main stack, heap and svcMapPhysicalMemory pages. Pages owned by
    /// services (shared memory), aliases and TLS are not charged, as in HOS.
    pub fn used_non_system_memory(&self) -> u64 {
        self.fixed_charge + self.heap_size as u64 + self.physical_mapped
    }

    /// The process memory limit minus its system resource reservation.
    pub fn total_non_system_memory(&self) -> u64 {
        APPLICATION_POOL_SIZE.saturating_sub(self.system_resource_size)
    }

    /// Modelled use of the system resource: one 4 KiB last-level page table
    /// per 2 MiB of physically mapped memory, bounded by the reservation.
    pub fn system_resource_used(&self) -> u64 {
        if self.system_resource_size == 0 {
            return 0;
        }
        (self.physical_mapped.div_ceil(0x20_0000) * 0x1000).min(self.system_resource_size)
    }

    /// Fails with LimitReached when charging `bytes` more would exceed the
    /// non-system memory limit.
    fn charge_check(&self, bytes: u64) -> Result<(), u64> {
        if self.used_non_system_memory().saturating_add(bytes) > self.total_non_system_memory() {
            Err(RESULT_LIMIT_REACHED)
        } else {
            Ok(())
        }
    }

    /// Validates an svc(Un)MapPhysicalMemory range: page aligned, nonempty,
    /// inside the alias region, and only for processes with a system resource.
    fn physical_range(&self, address: u64, size: u64) -> Result<u64, u64> {
        if address % 4096 != 0 {
            return Err(RESULT_INVALID_ADDRESS);
        }
        if size == 0 || size % 4096 != 0 {
            return Err(RESULT_INVALID_SIZE);
        }
        let end = address.checked_add(size).ok_or(RESULT_INVALID_MEMORY_REGION)?;
        if address < ALIAS_REGION_BASE || end > ALIAS_REGION_BASE + ALIAS_REGION_SIZE {
            return Err(RESULT_INVALID_MEMORY_REGION);
        }
        if self.system_resource_size == 0 {
            return Err(RESULT_INVALID_STATE);
        }
        Ok(end)
    }

    /// svcMapPhysicalMemory: back every unmapped page of the range with zeroed
    /// RW memory; already-mapped pages are kept. All-or-nothing on failure.
    fn map_physical_memory(&mut self, cpu: &mut Cpu, memory: &mut AddressSpace) {
        let (address, size) = (cpu.x[0], cpu.x[1]);
        cpu.x[0] = (|| {
            let end = match self.physical_range(address, size) {
                Ok(end) => end,
                Err(result) => return result,
            };
            let mut gaps = Vec::new();
            let mut cursor = address;
            while cursor < end {
                let block = memory.query_memory(cursor);
                let block_end = block.base_address.saturating_add(block.size).min(end);
                if block_end <= cursor {
                    return RESULT_INVALID_CURRENT_MEMORY;
                }
                if block.state == 0 {
                    gaps.push((cursor, block_end - cursor));
                } else if block.state != PHYSICAL_MEMORY_STATE {
                    return RESULT_INVALID_CURRENT_MEMORY;
                }
                cursor = block_end;
            }
            let needed: u64 = gaps.iter().map(|(_, length)| length).sum();
            if let Err(result) = self.charge_check(needed) {
                return result;
            }
            if needed > memory.remaining_budget() as u64 {
                return RESULT_OUT_OF_MEMORY;
            }
            let mut mapped: Vec<(u64, u64)> = Vec::new();
            for &(start, length) in &gaps {
                if memory
                    .map(start, vec![0; length as usize], Permissions::READ_WRITE)
                    .is_err()
                {
                    for &(done, done_length) in &mapped {
                        memory.unmap(done, done_length as usize).expect("mapped by this call");
                    }
                    return RESULT_OUT_OF_MEMORY;
                }
                let _ = memory.set_state(start, length as usize, PHYSICAL_MEMORY_STATE);
                mapped.push((start, length));
            }
            self.physical_mapped += mapped.iter().map(|(_, length)| length).sum::<u64>();
            0
        })();
    }

    /// svcUnmapPhysicalMemory: release the physically backed pages of the
    /// range. Any other kind of mapping inside it rejects the whole call.
    fn unmap_physical_memory(&mut self, cpu: &mut Cpu, memory: &mut AddressSpace) {
        let (address, size) = (cpu.x[0], cpu.x[1]);
        cpu.x[0] = (|| {
            let end = match self.physical_range(address, size) {
                Ok(end) => end,
                Err(result) => return result,
            };
            let mut backed = Vec::new();
            let mut cursor = address;
            while cursor < end {
                let block = memory.query_memory(cursor);
                let block_end = block.base_address.saturating_add(block.size).min(end);
                if block_end <= cursor {
                    return RESULT_INVALID_CURRENT_MEMORY;
                }
                match block.state {
                    0 => {}
                    PHYSICAL_MEMORY_STATE => backed.push((cursor, block_end - cursor)),
                    _ => return RESULT_INVALID_CURRENT_MEMORY,
                }
                cursor = block_end;
            }
            for &(start, length) in &backed {
                if memory.unmap(start, length as usize).is_err() {
                    return RESULT_INVALID_CURRENT_MEMORY;
                }
                self.physical_mapped -= length;
            }
            0
        })();
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
            || scheduler::overlaps_tls(address, end)
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
    /// Diagnostic description of a live handle: object kind, signal state and origin.
    pub fn describe_handle(&self, handle: u32) -> Option<String> {
        let object = self.handles.get(handle).ok()?;
        let kind = match &object {
            Object::Thread(t) => format!("thread #{}", t.id),
            Object::Process(_) => "process".into(),
            Object::TransferMemory(_) => "transfer memory".into(),
            Object::Session(s) => format!("session {}", short_type_name(s.borrow().name())),
            Object::WritableEvent(e) => format!("writable event (signaled={})", e.get()),
            Object::ReadableEvent(e) => format!("readable event (signaled={})", e.get()),
            Object::SharedMemory(_) => "shared memory".into(),
        };
        Some(match self.handle_origins.get(&handle) {
            Some(origin) => format!("{kind} from {origin}"),
            None => kind,
        })
    }
    pub fn thread_summaries(&self) -> Vec<ThreadSummary> {
        self.scheduler.thread_summaries()
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
    /// Most recent IPC messages, oldest first (bounded diagnostic ring).
    pub fn ipc_trace(&self) -> impl Iterator<Item = &IpcTrace> {
        self.ipc_trace.iter()
    }
    /// Most recent supervisor calls, oldest first (bounded diagnostic ring).
    pub fn svc_records(&self) -> impl Iterator<Item = &SvcRecord> {
        self.svc_records.iter()
    }
    /// Most recent memory-management supervisor calls, oldest first.
    pub fn memory_svc_records(&self) -> impl Iterator<Item = &MemorySvcRecord> {
        self.memory_svc_records.iter()
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
    /// Requests per (service, command), for diagnosing guest polling loops.
    pub fn ipc_histogram(&self) -> &BTreeMap<(&'static str, u32), u64> {
        &self.ipc_histogram
    }
    /// Service (or domain object) that received the most recent request.
    pub fn last_ipc_service(&self) -> &'static str {
        self.last_ipc_service
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
        let mut trace = IpcTrace {
            handle: cpu.x[0] as u32,
            ..IpcTrace::default()
        };
        let outcome = self.send_sync_traced(cpu, memory, &mut trace);
        trace.sequence = self.ipc_count;
        if outcome.is_err() {
            trace.result = None;
        }
        if self.ipc_trace.len() == IPC_TRACE_CAPACITY {
            self.ipc_trace.pop_front();
        }
        self.ipc_trace.push_back(trace);
        outcome
    }

    fn send_sync_traced(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut AddressSpace,
        trace: &mut IpcTrace,
    ) -> Result<(), KernelError> {
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
        // Only the first MAX_IPC_RECORDS messages are kept in full; later ones
        // are still counted and visible in the bounded trace ring.
        let record_ipc = self.ipc_records.len() < MAX_IPC_RECORDS;
        if record_ipc {
            self.ipc_records
                .try_reserve(1)
                .map_err(|_| KernelError::IpcTraceLimit)?;
        }
        trace.service = service.borrow().name();
        let mut input = [0; IPC_BUFFER_SIZE];
        memory
            .read(cpu.tpidrro_el0, &mut input)
            .map_err(KernelError::Memory)?;
        memory
            .check_access(cpu.tpidrro_el0, IPC_BUFFER_SIZE, Access::Write)
            .map_err(KernelError::Memory)?;
        self.domains.retain(|domain| domain.session.strong_count() != 0);
        for (index, word) in trace.words.iter_mut().enumerate() {
            *word = u32::from_le_bytes(input[index * 4..index * 4 + 4].try_into().expect("word"));
        }
        let domain_index = self
            .domains
            .iter()
            .position(|domain| domain.session.as_ptr() == Rc::as_ptr(&service));
        let mut request = emulator_ipc::parse_session_request(&input, domain_index.is_some())
            .map_err(KernelError::Ipc)?;
        self.ipc_count = self.ipc_count.saturating_add(1);
        self.last_ipc_cmd = request.command_id;
        trace.control = request.kind == CommandType::Control;
        trace.tipc = request.tipc;
        trace.domain_object = request.domain.as_ref().map(|domain| domain.object_id);
        trace.command_id = (!request.domain.as_ref().is_some_and(|domain| domain.close))
            .then_some(request.command_id);
        trace.request_bytes = request.data.len();
        trace.buffers = [
            request.send_buffers.len(),
            request.receive_buffers.len(),
            request.pointer_send.len(),
            request.pointer_receive.len(),
        ];
        trace.copy_handles = request.copy_handles.len();
        if request.pid.is_some() {
            request.pid = Some(1);
        } // Owned guest process ID, never the host PID.
        if request.kind == CommandType::Close {
            // The session is finished: drop its domain objects now. The guest
            // gets the kernel's session-closed result and no reply bytes, and
            // releases the handle itself with svcCloseHandle.
            if let Some(index) = domain_index {
                self.domains.swap_remove(index);
            }
            trace.command_id = None;
            trace.result = Some(RESULT_SESSION_CLOSED as u32);
            cpu.x[0] = RESULT_SESSION_CLOSED;
            return Ok(());
        }
        if request.kind == CommandType::Control {
            // 0: ConvertCurrentObjectToDomain, 1: CopyFromCurrentDomain(u32 id),
            // 2: CloneCurrentObject, 3: QueryPointerBufferSize,
            // 4: CloneCurrentObjectEx(u32 tag).
            let payload = match request.command_id {
                0 | 2 | 3 => 0,
                1 | 4 => 4,
                other => return Err(KernelError::UnsupportedControl(other)),
            };
            if request.data.len() != payload || request.pid.is_some() {
                return Err(KernelError::Ipc(IpcError::UnsupportedLayout));
            }
            let mut staged = Vec::new();
            let mut data = Vec::new();
            let mut convert = false;
            match request.command_id {
                0 => {
                    // The session's own object becomes domain object 1.
                    if domain_index.is_some() {
                        return Err(KernelError::UnsupportedControl(0));
                    }
                    convert = true;
                    data = 1_u32.to_le_bytes().to_vec();
                }
                1 => {
                    let id = u32::from_le_bytes(request.data[..4].try_into().expect("checked"));
                    let object = domain_index
                        .and_then(|index| self.domains[index].objects.get(&id))
                        .ok_or(KernelError::InvalidDomainObject(id))?;
                    staged.push(Object::Session(Rc::clone(object)));
                }
                3 => data = u32::from(POINTER_BUFFER_SIZE).to_le_bytes().to_vec(),
                _ => staged.push(Object::Session(Rc::clone(&service))),
            }
            let pending = self.handles.stage(staged).map_err(KernelError::Handle)?;
            let encoded = encode_reply(0, &data, &pending.ids).map_err(KernelError::Ipc)?;
            if convert {
                self.domains.push(Domain {
                    session: Rc::downgrade(&service),
                    objects: [(1, Rc::clone(&service))].into(),
                    next_id: 2,
                });
            }
            memory
                .write(cpu.tpidrro_el0, &encoded)
                .map_err(KernelError::Memory)?;
            self.handles.commit(pending);
            trace.result = Some(0);
            cpu.x[0] = 0;
            if !record_ipc {
                return Ok(());
            }
            self.ipc_records.push(IpcRecord {
                handle,
                kind: request.kind,
                command_id: request.command_id,
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
        // On a domain session the message addresses one of its objects.
        let service = match (&request.domain, domain_index) {
            (Some(target), Some(index)) => {
                if !target.input_objects.is_empty() {
                    return Err(KernelError::InvalidDomainObject(target.input_objects[0]));
                }
                if target.close {
                    // Dropping an unknown ID is harmless and reported as success.
                    self.domains[index].objects.remove(&target.object_id);
                    let encoded = emulator_ipc::encode_session_reply(0, &[], &[], &[], Some(&[]))
                        .map_err(KernelError::Ipc)?;
                    memory
                        .write(cpu.tpidrro_el0, &encoded)
                        .map_err(KernelError::Memory)?;
                    trace.result = Some(0);
                    cpu.x[0] = 0;
                    return Ok(());
                }
                Rc::clone(
                    self.domains[index]
                        .objects
                        .get(&target.object_id)
                        .ok_or(KernelError::InvalidDomainObject(target.object_id))?,
                )
            }
            _ => service,
        };
        trace.service = service.borrow().name();
        self.last_ipc_service = trace.service;
        *self
            .ipc_histogram
            .entry((trace.service, request.command_id))
            .or_default() += 1;
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
        let mut reply = candidate
            .prepare(&request, &context)
            .map_err(KernelError::Service)?;
        // Raw data travels in whole words; sub-word outputs (bool, u8) are
        // zero-padded exactly as the wire format requires.
        reply.data.resize(reply.data.len().next_multiple_of(4), 0);
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
        // Interfaces returned on a domain session become domain objects;
        // kernel objects (events, shared memory) are always real handles.
        let in_domain = request.domain.as_ref().and(domain_index);
        // Kernel objects are staged first so their IDs lead `pending.ids`.
        let mut staged_objects = Vec::new();
        let mut sessions = Vec::new();
        let mut domain_objects = Vec::new();
        for moved in reply.moved_objects {
            match (moved.as_handle(), in_domain) {
                (Some(object), _) => staged_objects.push(object),
                (None, Some(_)) => {
                    domain_objects.push(Rc::new(std::cell::RefCell::new(moved)))
                }
                (None, None) => sessions.push(Object::session(moved)),
            }
        }
        let copied_count = staged_objects.len();
        staged_objects.append(&mut sessions);
        let domain_ids: Option<Vec<u32>> = match in_domain {
            Some(index) => {
                let domain = &self.domains[index];
                let end = domain
                    .next_id
                    .checked_add(domain_objects.len() as u32)
                    .filter(|_| {
                        domain.objects.len() + domain_objects.len() <= MAX_DOMAIN_OBJECTS
                    })
                    .ok_or(KernelError::Handle(handles::HandleError::Limit))?;
                Some((domain.next_id..end).collect())
            }
            None => None,
        };
        let pending = self
            .handles
            .stage(staged_objects)
            .map_err(KernelError::Handle)?;
        // Events and shared memory reach the guest as copy handles, sessions
        // as move handles. TIPC (sm:) only ever returns sessions.
        let (copied_ids, moved_ids) = pending.ids.split_at(copied_count);
        let encoded = if request.tipc {
            if copied_count != 0 {
                return Err(KernelError::Ipc(IpcError::UnsupportedLayout));
            }
            emulator_ipc::encode_tipc_reply(reply.result, &reply.data, moved_ids)
        } else {
            emulator_ipc::encode_session_reply(
                reply.result,
                &reply.data,
                copied_ids,
                moved_ids,
                domain_ids.as_deref(),
            )
        }
        .map_err(KernelError::Ipc)?;
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
        let origin = format!("{} cmd {}", short_type_name(candidate.name()), request.command_id);
        for id in &pending.ids {
            self.handle_origins.insert(*id, origin.clone());
        }
        self.handles.commit(pending);
        *service.borrow_mut() = candidate;
        if let (Some(index), Some(ids)) = (in_domain, domain_ids) {
            let domain = &mut self.domains[index];
            for (id, object) in ids.into_iter().zip(domain_objects) {
                domain.objects.insert(id, object);
                domain.next_id = id + 1;
            }
        }
        trace.result = Some(reply.result);
        cpu.x[0] = 0;
        if !record_ipc {
            return Ok(());
        }
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
        for id in &pending.ids {
            self.handle_origins.insert(*id, "svcCreateEvent".into());
        }
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
        // Applications may use cores 0..=2; -2 selects the process default.
        if !(0..3).contains(&core) && core != -2 {
            return Err(KernelError::UnsupportedProcessor(core));
        }
        let object = ThreadObject::new(id);
        let pending = self
            .handles
            .stage(vec![Object::Thread(object.clone())])
            .map_err(KernelError::Handle)?;
        let tls_page = scheduler::thread_tls_page(id);
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
            cpu.x[0] = RESULT_INVALID_POINTER;
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
            cpu.x[0] = RESULT_INVALID_POINTER;
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
            cpu.x[0] = RESULT_INVALID_POINTER;
            return Ok(());
        }
        if in_pageinfo_addr != 0 && in_pageinfo_addr % 4 == 0 {
            let _ = memory.write_u32(in_pageinfo_addr, 0);
        }
        cpu.x[0] = 0;
        cpu.x[1] = 0;
        Ok(())
    }

    /// svcMapMemory: alias read-write source pages at an unmapped destination
    /// (thread stacks). The source stays inaccessible until svcUnmapMemory.
    fn map_memory(&mut self, cpu: &mut Cpu, memory: &mut AddressSpace) {
        let (destination, source) = (cpu.x[0], cpu.x[1]);
        cpu.x[0] = (|| {
            let Some(size) = alias_size(destination, source, cpu.x[2]) else {
                return if destination % 4096 != 0 || source % 4096 != 0 {
                    RESULT_INVALID_ADDRESS
                } else {
                    RESULT_INVALID_SIZE
                };
            };
            if self.memory_aliases.len() >= MAX_MEMORY_ALIASES {
                return RESULT_OUT_OF_MEMORY;
            }
            let Ok(view) = memory.pin(source, size, Permissions::READ_WRITE) else {
                return RESULT_INVALID_CURRENT_MEMORY;
            };
            if memory
                .map_alias(destination, &view, Permissions::READ_WRITE)
                .is_err()
            {
                return RESULT_INVALID_CURRENT_MEMORY;
            }
            drop(view);
            if memory.protect(source, size, Permissions::NONE).is_err() {
                memory
                    .unmap(destination, size)
                    .expect("alias mapped by this call");
                return RESULT_INVALID_CURRENT_MEMORY;
            }
            let _ = memory.set_state(destination, size, 0x0b);
            self.memory_aliases.push((destination, source, size));
            0
        })();
    }

    /// svcUnmapMemory: only an exact, live svcMapMemory pair can be undone.
    fn unmap_memory(&mut self, cpu: &mut Cpu, memory: &mut AddressSpace) {
        let (destination, source) = (cpu.x[0], cpu.x[1]);
        cpu.x[0] = (|| {
            let Some(size) = alias_size(destination, source, cpu.x[2]) else {
                return RESULT_INVALID_SIZE;
            };
            let Some(index) = self
                .memory_aliases
                .iter()
                .position(|alias| *alias == (destination, source, size))
            else {
                return RESULT_INVALID_CURRENT_MEMORY;
            };
            if memory.unmap(destination, size).is_err() {
                return RESULT_INVALID_CURRENT_MEMORY;
            }
            memory
                .protect(source, size, Permissions::READ_WRITE)
                .expect("source hidden by svcMapMemory stays committed");
            self.memory_aliases.swap_remove(index);
            0
        })();
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
        if size as u64 > HEAP_REGION_SIZE {
            cpu.x[0] = RESULT_OUT_OF_MEMORY;
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
            if let Err(result) = self.charge_check(additional as u64) {
                cpu.x[0] = result;
                return Ok(());
            }
            if additional > memory.remaining_budget() {
                cpu.x[0] = RESULT_OUT_OF_MEMORY;
                return Ok(());
            }
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
        self.scheduler.request_reschedule();
        if self.exited {
            return Err(KernelError::ProcessAlreadyExited);
        }
        // SVC entry/return is an exception round trip: the monitor is cleared.
        cpu.clear_exclusive();
        self.svc_count = self.svc_count.saturating_add(1);
        self.last_svc_id = id;
        self.last_svc_pc = cpu.pc;
        let mut record = SvcRecord {
            sequence: self.svc_count,
            id,
            pc: cpu.pc,
            arguments: [cpu.x[0], cpu.x[1], cpu.x[2], cpu.x[3]],
            results: None,
        };
        let outcome = self.dispatch_service_call(id, cpu, memory);
        if outcome.is_ok() {
            record.results = Some([cpu.x[0], cpu.x[1], cpu.x[2]]);
        }
        if self.svc_records.len() == SVC_TRACE_CAPACITY {
            self.svc_records.pop_front();
        }
        self.svc_records.push_back(record);
        if is_memory_svc(id, record.arguments[1]) {
            if self.memory_svc_records.len() == MEMORY_SVC_TRACE_CAPACITY {
                self.memory_svc_records.pop_front();
            }
            self.memory_svc_records.push_back(MemorySvcRecord {
                call: record,
                mapped_after: memory.mapped_bytes() as u64,
            });
        }
        outcome
    }

    fn dispatch_service_call(
        &mut self,
        id: u16,
        cpu: &mut Cpu,
        memory: &mut AddressSpace,
    ) -> Result<KernelEvent, KernelError> {
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
                    cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_SET_MEMORY_ATTRIBUTE => {
                cpu.x[0] = 0;
                Ok(KernelEvent::Continue)
            }
            SVC_MAP_MEMORY => {
                self.map_memory(cpu, memory);
                Ok(KernelEvent::Continue)
            }
            SVC_UNMAP_MEMORY => {
                self.unmap_memory(cpu, memory);
                Ok(KernelEvent::Continue)
            }
            SVC_QUERY_MEMORY => {
                self.query_memory(cpu, memory)?;
                Ok(KernelEvent::Continue)
            }
            SVC_MAP_PHYSICAL_MEMORY => {
                self.map_physical_memory(cpu, memory);
                Ok(KernelEvent::Continue)
            }
            SVC_UNMAP_PHYSICAL_MEMORY => {
                self.unmap_physical_memory(cpu, memory);
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
                        cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
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
                if memory.read_u32(mutex_addr).is_err() || memory.read_u32(key_addr).is_err() {
                    cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
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
                // The key word tells userland a waiter exists; without it the
                // guest's condition-variable signal skips svcSignalProcessWideKey.
                if memory.write_u32(key_addr, 1).is_err() {
                    cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
                    return Ok(KernelEvent::Continue);
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
            SVC_WAIT_FOR_ADDRESS => {
                let addr = cpu.x[0];
                let kind = cpu.x[1] as u32;
                let value = cpu.x[2] as i32;
                let timeout_ns = cpu.x[3] as i64;
                if addr & 3 != 0 {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                if kind > 2 {
                    cpu.x[0] = RESULT_INVALID_ENUM_VALUE;
                    return Ok(KernelEvent::Continue);
                }
                let current = match memory.read_u32(addr) {
                    Ok(v) => v as i32,
                    Err(_) => {
                        cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
                        return Ok(KernelEvent::Continue);
                    }
                };
                // 0: WaitIfLessThan, 1: DecrementAndWaitIfLessThan, 2: WaitIfEqual.
                let should_wait = if kind == 2 { current == value } else { current < value };
                if !should_wait {
                    cpu.x[0] = RESULT_INVALID_STATE;
                    return Ok(KernelEvent::Continue);
                }
                if kind == 1 && memory.write_u32(addr, current.wrapping_sub(1) as u32).is_err() {
                    cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
                    return Ok(KernelEvent::Continue);
                }
                if timeout_ns == 0 {
                    cpu.x[0] = RESULT_TIMEOUT;
                    return Ok(KernelEvent::Continue);
                }
                let signal = self.sync.wait_address(self.scheduler.current(), addr);
                if self.scheduler.wait(vec![WaitTarget(signal)], timeout_ns).is_err() {
                    cpu.x[0] = RESULT_INVALID_STATE;
                } else {
                    cpu.x[0] = 0;
                }
                Ok(KernelEvent::Continue)
            }
            SVC_SIGNAL_TO_ADDRESS => {
                let addr = cpu.x[0];
                let kind = cpu.x[1] as u32;
                let value = cpu.x[2] as i32;
                let count = cpu.x[3] as i32;
                if addr & 3 != 0 {
                    cpu.x[0] = RESULT_INVALID_ADDRESS;
                    return Ok(KernelEvent::Continue);
                }
                if kind > 2 {
                    cpu.x[0] = RESULT_INVALID_ENUM_VALUE;
                    return Ok(KernelEvent::Continue);
                }
                if kind != 0 {
                    let current = match memory.read_u32(addr) {
                        Ok(v) => v as i32,
                        Err(_) => {
                            cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
                            return Ok(KernelEvent::Continue);
                        }
                    };
                    if current != value {
                        cpu.x[0] = RESULT_INVALID_STATE;
                        return Ok(KernelEvent::Continue);
                    }
                    // 1: SignalAndIncrementIfEqual.
                    // 2: SignalAndModifyByWaitingCountIfEqual.
                    let new_value = if kind == 1 {
                        value.wrapping_add(1)
                    } else {
                        let waiters = self.sync.address_waiter_count(addr);
                        if waiters == 0 {
                            value.wrapping_add(1)
                        } else if count <= 0 {
                            value.wrapping_sub(2)
                        } else if waiters <= count as usize {
                            value.wrapping_sub(1)
                        } else {
                            value
                        }
                    };
                    if memory.write_u32(addr, new_value as u32).is_err() {
                        cpu.x[0] = RESULT_INVALID_CURRENT_MEMORY;
                        return Ok(KernelEvent::Continue);
                    }
                }
                self.sync.signal_address(addr, count);
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
                    2 => ALIAS_REGION_BASE, // InfoType_AliasRegionAddress
                    3 => ALIAS_REGION_SIZE, // InfoType_AliasRegionSize
                    4 => self.heap_base, // InfoType_HeapRegionAddress
                    5 => HEAP_REGION_SIZE, // InfoType_HeapRegionSize (6 GiB, 39-bit layout)
                    6 => APPLICATION_POOL_SIZE, // InfoType_TotalMemorySize
                    // InfoType_UsedMemorySize: charged pages plus the system resource.
                    7 => self.used_non_system_memory() + self.system_resource_size,
                    8 => 0, // InfoType_DebuggerAttached
                    9 => 0, // InfoType_ResourceLimit
                    10 => 0, // InfoType_IdleTickCount
                    11 => 0x0123_4567_89ab_cdef, // InfoType_RandomEntropy
                    12 => self.aslr_base, // InfoType_AslrRegionAddress
                    13 => self.aslr_size, // InfoType_AslrRegionSize
                    14 => 0x70_0000_0000, // InfoType_StackRegionAddress
                    15 => 0x1000_0000, // InfoType_StackRegionSize
                    16 => self.system_resource_size, // InfoType_SystemResourceSizeTotal
                    17 => self.system_resource_used(), // InfoType_SystemResourceSizeUsed
                    18 => self.program_id, // InfoType_ProgramId
                    19 => 0, // InfoType_InitialProcessIdRange
                    // InfoType_UserExceptionContextAddress: Horizon reserves a
                    // 0x200-byte TLS slot per process. Thread TLS starts at page
                    // offset 0x200, so the main thread page's first slot is free
                    // and always mapped; returning 0 made guests write to null.
                    20 => USER_EXCEPTION_CONTEXT,
                    21 => self.total_non_system_memory(), // InfoType_TotalNonSystemMemorySize
                    22 => self.used_non_system_memory(), // InfoType_UsedNonSystemMemorySize
                    23 => 1, // InfoType_IsApplication (true)
                    24 => 128, // InfoType_FreeThreadCount
                    25 => 0, // InfoType_ThreadTickCount
                    26 => 1, // InfoType_IsSvcPermitted
                    27 => 0, // InfoType_IoRegionHint
                    28 => 0, // InfoType_AliasRegionExtraSize
                    _ => {
                        cpu.x[0] = RESULT_INVALID_ENUM_VALUE;
                        return Ok(KernelEvent::Continue);
                    }
                };
                if out_ptr != 0 && out_ptr % 4 == 0 {
                    let _ = memory.write_u64(out_ptr, val);
                }
                cpu.x[0] = 0;
                cpu.x[1] = val;
                Ok(KernelEvent::Continue)
            }
            SVC_BREAK => {
                let reason = cpu.x[0] as u32;
                let info1 = cpu.x[1];
                let info2 = cpu.x[2];
                let pc = cpu.pc;
                let msg = format!("guest svcBreak: reason={reason:#x}, info1={info1:#x}, info2={info2:#x} at pc={pc:#x}\n");
                self.debug_records.push(msg.into_bytes());
                // Only a notification break (bit 31) returns to the guest;
                // any other break is the guest aborting itself.
                if reason >> 31 == 0 {
                    return Err(KernelError::GuestBreak {
                        reason,
                        info1,
                        info2,
                    });
                }
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
                let handle = cpu.x[0] as u32;
                cpu.x[0] = if self.handles.close(handle).is_ok() {
                    self.handle_origins.remove(&handle);
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

fn short_type_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}
