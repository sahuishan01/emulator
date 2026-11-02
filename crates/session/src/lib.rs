//! Composition of original NRO loading, CPU execution and the minimal kernel.

use emulator_a64::{Cpu, ExecutionError, StepEvent};
use emulator_kernel::{homebrew_config, Kernel, KernelError, KernelEvent, ScheduleOutcome};
use emulator_loader::{LoadError, LoadedImage, NroImage};
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

pub struct Session {
    cpu: Cpu,
    memory: AddressSpace,
    kernel: Kernel,
    image: LoadedImage,
    state: SessionState,
    instructions: u64,
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
        memory
            .map(TLS_PAGE, vec![0; 4096], Permissions::READ_WRITE)
            .map_err(SessionError::Memory)?;
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
            let event = match self.cpu.step(&mut self.memory) {
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
                match self
                    .kernel
                    .dispatch(immediate, &mut self.cpu, &mut self.memory)
                {
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
