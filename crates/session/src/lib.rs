//! Composition of original NRO loading, CPU execution and the minimal kernel.

use emulator_a64::{Cpu, ExecutionError, StepEvent};
use emulator_kernel::{homebrew_config, Kernel, KernelError, KernelEvent, ScheduleOutcome};
use emulator_loader::storage::ReadOnlyStorage;
use emulator_loader::{LoadError, LoadedImage, NroImage, NsoProgram};
use emulator_memory::{AddressSpace, MemoryError, Permissions};
use std::fmt;

pub const LOAD_BASE: u64 = 0x10_0000;
pub const CONFIG_BASE: u64 = 0x7fff_0000;
pub const STACK_BASE: u64 = 0x8000_0000;
pub const STACK_SIZE: usize = 64 * 1024;
pub const TLS_PAGE: u64 = emulator_kernel::THREAD_TLS_ARENA;
pub const TLS_BASE: u64 = TLS_PAGE + 0x200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Loaded,
    Running,
    Paused,
    Blocked,
    Exited,
    Faulted,
}

#[derive(Debug)]
pub enum SessionError {
    Load(LoadError),
    Memory(MemoryError),
    Cpu(ExecutionError),
    Kernel(KernelError),
    InvalidState(SessionState),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => error.fmt(f),
            Self::Memory(error) => error.fmt(f),
            Self::Cpu(error) => error.fmt(f),
            Self::Kernel(error) => error.fmt(f),
            Self::InvalidState(state) => write!(f, "cannot execute session in state {state:?}"),
        }
    }
}
impl std::error::Error for SessionError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Exited,
    BudgetExhausted,
    Blocked,
}

/// Extra host budget above the guest-visible application pool.
const GUEST_MEMORY_HEADROOM: u64 = 0x1000_0000;

pub struct Session {
    cpu: Cpu,
    memory: AddressSpace,
    kernel: Kernel,
    image: LoadedImage,
    state: SessionState,
    instructions: u64,
    watch_log: Vec<WatchRecord>,
    trace_pcs: Vec<u64>,
    trace_log: Vec<TraceRecord>,
}

/// Register snapshot taken when execution reaches a traced PC (debug aid).
#[derive(Clone, Debug)]
pub struct TraceRecord {
    pub instruction: u64,
    pub thread: usize,
    pub pc: u64,
    pub lr: u64,
    pub sp: u64,
    pub x: [u64; 29],
    /// Return addresses from the x29 frame-pointer chain (up to 12).
    pub frames: Vec<u64>,
}

/// A watched write attributed to the instruction (or SVC) that caused it.
#[derive(Clone, Debug)]
pub struct WatchRecord {
    pub instruction: u64,
    pub thread: usize,
    pub pc: u64,
    pub lr: u64,
    pub address: u64,
    pub bytes: Vec<u8>,
}

impl Session {
    pub fn from_nro(bytes: &[u8]) -> Result<Self, SessionError> {
        Self::load(bytes, emulator_services::ServiceManager::default())
    }
    pub fn from_nro_with_graphics(
        bytes: &[u8],
        graphics: std::rc::Rc<dyn emulator_gpu::GraphicsBackend>,
    ) -> Result<Self, SessionError> {
        Self::load(
            bytes,
            emulator_services::ServiceManager::with_graphics(graphics),
        )
    }

    pub fn from_nso_program(program: &NsoProgram) -> Result<Self, SessionError> {
        Self::from_nso_program_with_services(program, emulator_services::ServiceManager::default())
    }

    pub fn from_exefs(
        program: &emulator_loader::exefs::ExeFsProgram,
    ) -> Result<Self, SessionError> {
        Self::from_exefs_with_services(program, emulator_services::ServiceManager::default())
    }

    pub fn from_exefs_with_services(
        program: &emulator_loader::exefs::ExeFsProgram,
        service_manager: emulator_services::ServiceManager,
    ) -> Result<Self, SessionError> {
        // Host budget: the application pool reported to the guest plus headroom for
        // pages HOS does not charge to it (shared memory, TLS, service buffers).
        // Backing is zero-filled on demand, so untouched guest memory costs no RAM.
        let mut memory = AddressSpace::with_limit(
            (emulator_kernel::APPLICATION_POOL_SIZE + GUEST_MEMORY_HEADROOM) as usize,
        );
        let mut nso_program = NsoProgram::default();
        program
            .load_into(&mut nso_program, &mut memory, LOAD_BASE)
            .map_err(SessionError::Load)?;
        // Everything mapped so far is the loaded image (code, data, bss).
        let image_bytes = memory.mapped_bytes() as u64;

        let main_module = nso_program
            .modules()
            .iter()
            .find(|m| m.name == "rtld")
            .or_else(|| nso_program.modules().iter().find(|m| m.name == "main"))
            .or_else(|| nso_program.modules().first())
            .ok_or(SessionError::Load(LoadError::Invalid(
                "NSO program contains no modules",
            )))?;
        let image = main_module.image;

        let stack_size = program
            .npdm
            .as_ref()
            .map(|meta| meta.main_thread_stack_size as usize)
            .filter(|&size| size >= 0x4000)
            .unwrap_or(STACK_SIZE);
        let stack_size = (stack_size + 4095) & !4095;

        let mut config = homebrew_config();
        config.resize(4096, 0);
        memory
            .map(CONFIG_BASE, config, Permissions::READ)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE - 4096, 4096)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE + stack_size as u64, 4096)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE, stack_size)
            .map_err(SessionError::Memory)?;
        memory
            .commit(STACK_BASE, vec![0; stack_size], Permissions::READ_WRITE)
            .map_err(SessionError::Memory)?;
        let _ = memory.set_state(STACK_BASE, stack_size, 0x0B);
        memory
            .map(TLS_PAGE, vec![0; 4096], Permissions::READ_WRITE)
            .map_err(SessionError::Memory)?;
        let _ = memory.set_state(TLS_PAGE, 4096, 0x0C);
        memory
            .write_u32(TLS_BASE + 0x110, emulator_kernel::MAIN_THREAD_HANDLE as u32)
            .map_err(SessionError::Memory)?;
        let mut cpu = Cpu::new(image.entry, STACK_BASE + stack_size as u64);
        cpu.tpidrro_el0 = TLS_BASE;
        cpu.x[0] = 0;
        cpu.x[1] = u64::from(emulator_kernel::MAIN_THREAD_HANDLE);
        let mut kernel = Kernel::default();
        kernel.set_aslr_region(LOAD_BASE, 0x7f_ff00_0000);
        kernel.set_fixed_memory_charge(image_bytes + stack_size as u64);
        if let Some(meta) = &program.npdm {
            kernel.set_program_id(meta.title_id);
            kernel.set_system_resource_size(u64::from(meta.system_resource_size));
        }
        kernel
            .register_named_port("sm:", Box::new(service_manager))
            .map_err(SessionError::Kernel)?;
        Ok(Self {
            cpu,
            memory,
            kernel,
            image,
            state: SessionState::Loaded,
            instructions: 0,
            watch_log: Vec::new(),
            trace_pcs: Vec::new(),
            trace_log: Vec::new(),
        })
    }

    pub fn from_nca(
        archive: &[u8],
        keys: &emulator_crypto::keys::KeyFile,
    ) -> Result<Self, SessionError> {
        Self::from_nca_with_services(archive, keys, emulator_services::ServiceManager::default())
    }

    pub fn from_nca_with_services(
        archive: &[u8],
        keys: &emulator_crypto::keys::KeyFile,
        mut service_manager: emulator_services::ServiceManager,
    ) -> Result<Self, SessionError> {
        let header =
            emulator_loader::nca::decrypt_nca3_header(archive, keys).map_err(SessionError::Load)?;
        header
            .verify_fs_header_hashes()
            .map_err(SessionError::Load)?;
        if let Ok(romfs) = header.open_romfs(archive, keys) {
            service_manager.set_romfs(romfs);
        }
        let exefs = header
            .open_exefs(archive, keys)
            .map_err(SessionError::Load)?;
        Self::from_exefs_with_services(&exefs, service_manager)
    }

    pub fn from_nsp(
        archive: &[u8],
        keys: &mut emulator_crypto::keys::KeyFile,
    ) -> Result<Self, SessionError> {
        Self::from_nsp_with_services(archive, keys, emulator_services::ServiceManager::default())
    }

    pub fn from_nsp_with_services(
        archive: &[u8],
        keys: &mut emulator_crypto::keys::KeyFile,
        service_manager: emulator_services::ServiceManager,
    ) -> Result<Self, SessionError> {
        Self::from_nsp_shared(std::rc::Rc::new(archive.to_vec()), keys, service_manager)
    }

    /// [`Self::from_nsp_shared`] with the default services.
    pub fn from_nsp_owned(
        archive: emulator_loader::storage::SharedBytes,
        keys: &mut emulator_crypto::keys::KeyFile,
    ) -> Result<Self, SessionError> {
        Self::from_nsp_shared(archive, keys, emulator_services::ServiceManager::default())
    }

    /// Loads an NSP without copying it: the program's RomFS stays in the shared
    /// container and is decrypted lazily per IStorage read.
    pub fn from_nsp_shared(
        shared: emulator_loader::storage::SharedBytes,
        keys: &mut emulator_crypto::keys::KeyFile,
        mut service_manager: emulator_services::ServiceManager,
    ) -> Result<Self, SessionError> {
        let archive: &[u8] = (*shared).as_ref();
        let nsp = emulator_loader::nsp::NspArchive::parse(archive).map_err(SessionError::Load)?;
        let (header, prog_nca) = nsp.open_program(keys).map_err(SessionError::Load)?;
        // `prog_nca` borrows from `archive`: recover its offset in the container.
        let nca_offset = (prog_nca.as_ptr() as usize - archive.as_ptr() as usize) as u64;
        match header.open_romfs_storage(std::rc::Rc::clone(&shared), nca_offset, keys) {
            Ok(storage) => {
                // Small RomFS images also get the parsed filesystem view.
                if storage.size() <= 128 * 1024 * 1024 {
                    if let Ok(romfs) = header.open_romfs(prog_nca, keys) {
                        service_manager.set_romfs(romfs);
                    }
                }
                service_manager.set_data_storage(std::rc::Rc::new(storage));
            }
            Err(emulator_loader::LoadError::Invalid(
                "NCA3 archive does not contain a RomFS section",
            )) => {}
            Err(error) => return Err(SessionError::Load(error)),
        }
        let exefs = header
            .open_exefs(prog_nca, keys)
            .map_err(SessionError::Load)?;
        Self::from_exefs_with_services(&exefs, service_manager)
    }

    pub fn from_nso_program_with_services(
        program: &NsoProgram,
        service_manager: emulator_services::ServiceManager,
    ) -> Result<Self, SessionError> {
        let main_module = program
            .modules()
            .iter()
            .find(|m| m.name == "main")
            .or_else(|| program.modules().first())
            .ok_or(SessionError::Load(LoadError::Invalid(
                "NSO program contains no modules",
            )))?;
        let image = main_module.image;
        let mut memory = AddressSpace::default();
        let mut config = homebrew_config();
        config.resize(4096, 0);
        memory
            .map(CONFIG_BASE, config, Permissions::READ)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE - 4096, 4096)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE + STACK_SIZE as u64, 4096)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE, STACK_SIZE)
            .map_err(SessionError::Memory)?;
        memory
            .commit(STACK_BASE, vec![0; STACK_SIZE], Permissions::READ_WRITE)
            .map_err(SessionError::Memory)?;
        let _ = memory.set_state(STACK_BASE, STACK_SIZE, 0x0B);
        memory
            .map(TLS_PAGE, vec![0; 4096], Permissions::READ_WRITE)
            .map_err(SessionError::Memory)?;
        let _ = memory.set_state(TLS_PAGE, 4096, 0x0C);
        let mut cpu = Cpu::new(image.entry, STACK_BASE + STACK_SIZE as u64);
        cpu.tpidrro_el0 = TLS_BASE;
        cpu.x[0] = CONFIG_BASE;
        cpu.x[1] = u64::MAX;
        let mut kernel = Kernel::default();
        kernel
            .register_named_port("sm:", Box::new(service_manager))
            .map_err(SessionError::Kernel)?;
        Ok(Self {
            cpu,
            memory,
            kernel,
            image,
            state: SessionState::Loaded,
            instructions: 0,
            watch_log: Vec::new(),
            trace_pcs: Vec::new(),
            trace_log: Vec::new(),
        })
    }
    fn load(
        bytes: &[u8],
        service_manager: emulator_services::ServiceManager,
    ) -> Result<Self, SessionError> {
        let nro = NroImage::parse(bytes).map_err(SessionError::Load)?;
        let mut memory = AddressSpace::default();
        let image = nro
            .map_into(&mut memory, LOAD_BASE)
            .map_err(SessionError::Load)?;
        let mut config = homebrew_config();
        config.resize(4096, 0);
        memory
            .map(CONFIG_BASE, config, Permissions::READ)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE - 4096, 4096)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE + STACK_SIZE as u64, 4096)
            .map_err(SessionError::Memory)?;
        memory
            .reserve(STACK_BASE, STACK_SIZE)
            .map_err(SessionError::Memory)?;
        memory
            .commit(STACK_BASE, vec![0; STACK_SIZE], Permissions::READ_WRITE)
            .map_err(SessionError::Memory)?;
        let _ = memory.set_state(STACK_BASE, STACK_SIZE, 0x0B);
        memory
            .map(TLS_PAGE, vec![0; 4096], Permissions::READ_WRITE)
            .map_err(SessionError::Memory)?;
        let _ = memory.set_state(TLS_PAGE, 4096, 0x0C);
        let mut cpu = Cpu::new(image.entry, STACK_BASE + STACK_SIZE as u64);
        cpu.tpidrro_el0 = TLS_BASE;
        cpu.x[0] = CONFIG_BASE;
        cpu.x[1] = u64::MAX;
        let mut kernel = Kernel::default();
        kernel
            .register_named_port("sm:", Box::new(service_manager))
            .map_err(SessionError::Kernel)?;
        Ok(Self {
            cpu,
            memory,
            kernel,
            image,
            state: SessionState::Loaded,
            instructions: 0,
            watch_log: Vec::new(),
            trace_pcs: Vec::new(),
            trace_log: Vec::new(),
        })
    }

    pub fn clear_ipc_trace(&mut self) {
        self.kernel.clear_ipc_trace();
    }
    pub fn state(&self) -> SessionState {
        self.state
    }
    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }
    pub fn memory(&self) -> &AddressSpace {
        &self.memory
    }
    pub fn image(&self) -> LoadedImage {
        self.image
    }
    pub fn instructions(&self) -> u64 {
        self.instructions
    }
    pub fn debug_records(&self) -> &[Vec<u8>] {
        self.kernel.debug_records()
    }
    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    /// Enables a debug write watchpoint over `[start, start + len)`.
    pub fn set_write_watch(&mut self, start: u64, len: u64) {
        self.memory.set_write_watch(Some((start, len)));
    }
    pub fn watch_log(&self) -> &[WatchRecord] {
        &self.watch_log
    }
    /// Records a register snapshot every time one of `pcs` is about to execute
    /// (capped at 4096 records).
    pub fn set_pc_trace(&mut self, pcs: Vec<u64>) {
        self.trace_pcs = pcs;
    }
    pub fn trace_log(&self) -> &[TraceRecord] {
        &self.trace_log
    }
    fn collect_trace(&mut self) {
        let pc = self.cpu.pc;
        if self.trace_log.len() >= 4096 || !self.trace_pcs.contains(&pc) {
            return;
        }
        let mut x = [0u64; 29];
        x.copy_from_slice(&self.cpu.x[..29]);
        let mut frames = Vec::new();
        let mut fp = self.cpu.x[29];
        while frames.len() < 12 && fp != 0 && fp % 16 == 0 {
            let (Ok(next), Ok(ret)) = (self.memory.read_u64(fp), self.memory.read_u64(fp + 8))
            else {
                break;
            };
            frames.push(ret);
            if next <= fp {
                break;
            }
            fp = next;
        }
        self.trace_log.push(TraceRecord {
            frames,
            instruction: self.instructions,
            thread: self.kernel.current_thread_id(),
            pc,
            lr: self.cpu.x[30],
            sp: self.cpu.sp,
            x,
        });
    }
    fn collect_watch(&mut self, pc: u64, lr: u64) {
        let hits = self.memory.take_watch_hits();
        if hits.is_empty() {
            return;
        }
        let thread = self.kernel.current_thread_id();
        for hit in hits {
            if self.watch_log.len() >= 4096 {
                break;
            }
            self.watch_log.push(WatchRecord {
                instruction: self.instructions,
                thread,
                pc,
                lr,
                address: hit.address,
                bytes: hit.bytes,
            });
        }
    }

    pub fn run(&mut self, budget: u64) -> Result<RunOutcome, SessionError> {
        if !matches!(
            self.state,
            SessionState::Loaded | SessionState::Paused | SessionState::Blocked
        ) {
            return Err(SessionError::InvalidState(self.state));
        }
        self.state = SessionState::Running;
        for _ in 0..budget {
            if self.kernel.prepare_execution(&mut self.cpu) == ScheduleOutcome::Blocked {
                self.state = SessionState::Blocked;
                return Ok(RunOutcome::Blocked);
            }
            if !self.trace_pcs.is_empty() {
                self.collect_trace();
            }
            let (pc, lr) = (self.cpu.pc, self.cpu.x[30]);
            let event = self.cpu.step(&mut self.memory);
            self.collect_watch(pc, lr);
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    self.state = SessionState::Faulted;
                    return Err(SessionError::Cpu(error));
                }
            };
            self.instructions = self.instructions.saturating_add(1);
            if let Err(error) = self.kernel.instruction_completed() {
                self.state = SessionState::Faulted;
                return Err(SessionError::Kernel(error));
            }
            if let StepEvent::SupervisorCall { immediate } = event {
                let result = self
                    .kernel
                    .dispatch(immediate, &mut self.cpu, &mut self.memory);
                self.collect_watch(pc, lr);
                match result {
                    Ok(KernelEvent::Continue) => {}
                    Ok(KernelEvent::ProcessExited) => {
                        self.state = SessionState::Exited;
                        return Ok(RunOutcome::Exited);
                    }
                    Err(error) => {
                        self.state = SessionState::Faulted;
                        return Err(SessionError::Kernel(error));
                    }
                }
            }
        }
        self.state = SessionState::Paused;
        Ok(RunOutcome::BudgetExhausted)
    }
}
