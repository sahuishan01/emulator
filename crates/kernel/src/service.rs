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
    BufferLimit,
    InvalidBufferCounts { send: usize, receive: usize },
    ReplyBufferOverlap,
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
    send: Vec<MemoryView>,
    receive: Vec<MemoryView>,
}
impl<'a> ServiceContext<'a> {
    pub(crate) fn new(memory: &'a AddressSpace, request: &Request) -> Result<Self, ServiceError> {
        let mut context = Self {
            memory,
            send: Vec::new(),
            receive: Vec::new(),
        };
        let mut total = 0_u64;
        let mut spans = 0;
        for (descriptors, views, permissions) in [
            (&request.send_buffers, &mut context.send, Permissions::READ),
            (
                &request.receive_buffers,
                &mut context.receive,
                Permissions::WRITE,
            ),
        ] {
            for descriptor in descriptors {
                total = total
                    .checked_add(descriptor.size)
                    .ok_or(ServiceError::BufferLimit)?;
                if descriptor.size == 0 || total > 8 * 1024 * 1024 {
                    return Err(ServiceError::BufferLimit);
                }
                let size =
                    usize::try_from(descriptor.size).map_err(|_| ServiceError::BufferLimit)?;
                let view = memory
                    .pin(descriptor.address, size, permissions)
                    .map_err(ServiceError::Memory)?;
                spans += view.backing_ranges().count();
                if spans > 4096 {
                    return Err(ServiceError::BufferLimit);
                }
                views.push(view);
            }
        }
        Ok(context)
    }
    pub(crate) fn validate_reply(&self, reply: &MemoryView) -> Result<(), ServiceError> {
        if self.receive.iter().any(|view| {
            view.backing_ranges().any(|a| {
                reply.backing_ranges().any(|b| {
                    a.id == b.id && a.offset < b.offset + b.length && b.offset < a.offset + a.length
                })
            })
        }) {
            return Err(ServiceError::ReplyBufferOverlap);
        }
        Ok(())
    }
    pub fn require_buffers(&self, send: usize, receive: usize) -> Result<(), ServiceError> {
        if self.send.len() != send || self.receive.len() != receive {
            return Err(self.buffer_error());
        }
        Ok(())
    }
    fn buffer_error(&self) -> ServiceError {
        ServiceError::InvalidBufferCounts {
            send: self.send.len(),
            receive: self.receive.len(),
        }
    }
    pub fn send_buffer(&self, index: usize) -> Result<MemoryView, ServiceError> {
        self.send
            .get(index)
            .cloned()
            .ok_or_else(|| self.buffer_error())
    }
    pub fn receive_buffer(&self, index: usize) -> Result<MemoryView, ServiceError> {
        self.receive
            .get(index)
            .cloned()
            .ok_or_else(|| self.buffer_error())
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
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(0, 0)?;
        self.dispatch(request)
    }
}
