//! Host service contract. Preparing on an isolated clone allows the kernel to
//! validate/encode output and stage handles before publishing service state.
use emulator_ipc::Request;
use emulator_memory::{AddressSpace, BackingRange, MemoryError, MemoryView, Permissions};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceError {
    UnsupportedCommand { service: &'static str, command: u32 },
    InvalidPayload { expected: usize, actual: usize },
    NotInitialized,
    AlreadyInitialized,
    MissingSenderIdentity,
    UnsupportedService([u8; 8]),
    ArithmeticOverflow,
    MissingMemoryContext,
    InvalidEffect,
    DeviceFault(String),
    Memory(MemoryError),
}
impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "guest service error: {self:?}")
    }
}
impl std::error::Error for ServiceError {}

#[derive(Debug, Default)]
pub struct ServiceReply {
    pub result: u32,
    pub data: Vec<u8>,
    pub moved_objects: Vec<Box<dyn Service>>,
    pub effect: Option<Box<dyn PreparedServiceEffect>>,
}

/// Read-only preparation context. Pins grant only existing guest rights.
/// No mutable address space or backing borrow escapes into a service.
pub struct ServiceContext<'a> {
    memory: &'a AddressSpace,
}
impl<'a> ServiceContext<'a> {
    pub(crate) fn new(memory: &'a AddressSpace) -> Self {
        Self { memory }
    }
    pub fn pin(
        &self,
        address: u64,
        size: usize,
        permissions: Permissions,
    ) -> Result<MemoryView, ServiceError> {
        self.memory
            .pin(address, size, permissions)
            .map_err(ServiceError::Memory)
    }
}
/// Trusted original services must keep preparation side-effect free. Report
/// every backing span commit can write. Commit cannot allocate/fail/call back
/// into the kernel or alter mappings; prepared pins provide all required rights.
/// Dropping an effect must cancel it without changing guest bytes or live state.
pub trait PreparedServiceEffect: fmt::Debug {
    fn write_ranges(&self) -> &[BackingRange];
    fn commit(self: Box<Self>);
}

pub trait Service: fmt::Debug {
    /// Must isolate mutable request state; do not share mutable internals with
    /// the original. Handle aliases are managed separately by the kernel.
    fn clone_for_request(&self) -> Box<dyn Service>;
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError>;
    fn prepare(
        &mut self,
        request: &Request,
        _context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        self.dispatch(request)
    }
}
