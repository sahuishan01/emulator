//! Host service contract. Preparing on an isolated clone allows the kernel to
//! validate/encode output and stage handles before publishing service state.
use crate::handles::{Object, ProcessIdentity, TransferMemory};
use emulator_ipc::Request;
use emulator_memory::{AddressSpace, BackingRange, MemoryError, MemoryView, Permissions};
use std::fmt;
use std::rc::Rc;

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
    InvalidCopyHandleCount(usize),
    InvalidTransferMemory,
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
    send: Vec<Option<MemoryView>>,
    receive: Vec<Option<MemoryView>>,
    pointer_send: Vec<Option<MemoryView>>,
    pointer_receive: Vec<Option<MemoryView>>,
    process: ProcessIdentity,
    copied: Vec<Object>,
    /// Guest virtual time when the request was sent (GetSystemTick's clock).
    time_ns: u64,
    timers: Rc<crate::timers::TimerQueue>,
}
impl<'a> ServiceContext<'a> {
    /// Guest virtual time in nanoseconds at the request.
    pub fn virtual_time_ns(&self) -> u64 {
        self.time_ns
    }
    pub(crate) fn with_virtual_time(mut self, time_ns: u64) -> Self {
        self.time_ns = time_ns;
        self
    }
    /// The kernel's timer queue. Arm timers from a committed effect, not
    /// while preparing, so failed requests leave no timers behind.
    pub fn timers(&self) -> &Rc<crate::timers::TimerQueue> {
        &self.timers
    }
    pub(crate) fn with_timers(mut self, timers: Rc<crate::timers::TimerQueue>) -> Self {
        self.timers = timers;
        self
    }
    pub(crate) fn new(
        memory: &'a AddressSpace,
        request: &Request,
        process: ProcessIdentity,
        copied: Vec<Object>,
    ) -> Result<Self, ServiceError> {
        let mut context = Self {
            memory,
            send: Vec::new(),
            receive: Vec::new(),
            pointer_send: Vec::new(),
            pointer_receive: Vec::new(),
            process,
            copied,
            time_ns: 0,
            timers: Rc::default(),
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
            (
                &request.pointer_send,
                &mut context.pointer_send,
                Permissions::READ,
            ),
            (
                &request.pointer_receive,
                &mut context.pointer_receive,
                Permissions::WRITE,
            ),
        ] {
            for descriptor in descriptors {
                total = total
                    .checked_add(descriptor.size)
                    .ok_or(ServiceError::BufferLimit)?;
                if total > 8 * 1024 * 1024 {
                    return Err(ServiceError::BufferLimit);
                }
                // Auto-select transfers leave the unused descriptor empty; it
                // grants nothing and is only usable through the auto accessors.
                if descriptor.size == 0 {
                    views.push(None);
                    continue;
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
                views.push(Some(view));
            }
        }
        Ok(context)
    }
    pub(crate) fn validate_reply(&self, reply: &MemoryView) -> Result<(), ServiceError> {
        if self.receive.iter().chain(&self.pointer_receive).flatten().any(|view| {
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
    pub fn require_copy_handles(&self, count: usize) -> Result<(), ServiceError> {
        if self.copied.len() != count {
            return Err(ServiceError::InvalidCopyHandleCount(self.copied.len()));
        }
        Ok(())
    }
    /// Exactly one copied handle, which must be the caller's own process.
    pub fn require_own_process(&self, index: usize) -> Result<(), ServiceError> {
        match (self.copied.len(), self.copied.get(index)) {
            (1, Some(Object::Process(process))) if process.same(&self.process) => Ok(()),
            _ => Err(ServiceError::InvalidCopyHandleCount(self.copied.len())),
        }
    }
    pub fn transfer_memory(
        &self,
        process_index: usize,
        transfer_index: usize,
    ) -> Result<Rc<TransferMemory>, ServiceError> {
        match (
            self.copied.get(process_index),
            self.copied.get(transfer_index),
        ) {
            (Some(Object::Process(process)), Some(Object::TransferMemory(memory)))
                if process.same(&self.process) && memory.owned_by(process) =>
            {
                Ok(Rc::clone(memory))
            }
            _ => Err(ServiceError::InvalidTransferMemory),
        }
    }
    /// Exactly one copied handle: transfer memory owned by the caller.
    pub fn sole_transfer_memory(&self) -> Result<Rc<TransferMemory>, ServiceError> {
        match self.copied.as_slice() {
            [Object::TransferMemory(memory)] if memory.owned_by(&self.process) => {
                Ok(Rc::clone(memory))
            }
            _ => Err(ServiceError::InvalidTransferMemory),
        }
    }
    pub fn require_buffers(&self, send: usize, receive: usize) -> Result<(), ServiceError> {
        if self.send.len() != send
            || self.receive.len() != receive
            || !self.pointer_send.is_empty()
            || !self.pointer_receive.is_empty()
        {
            return Err(self.buffer_error());
        }
        Ok(())
    }
    /// Exact descriptor counts: A (send), B (receive), X (pointer send) and C
    /// (pointer receive). An empty X descriptor still counts.
    pub fn require_exact_buffers(
        &self,
        send: usize,
        receive: usize,
        pointer_send: usize,
        pointer_receive: usize,
    ) -> Result<(), ServiceError> {
        if self.send.len() != send
            || self.receive.len() != receive
            || self.pointer_send.len() != pointer_send
            || self.pointer_receive.len() != pointer_receive
        {
            return Err(self.buffer_error());
        }
        Ok(())
    }
    /// Counts for auto-select (0x21/0x22) buffers: each logical buffer may
    /// arrive as a map-alias descriptor, a pointer descriptor, or both.
    /// Callers emit every declared buffer, so descriptors past the `send` /
    /// `receive` the command actually uses are accepted only when empty.
    pub fn require_auto_buffers(&self, send: usize, receive: usize) -> Result<(), ServiceError> {
        fn fits(alias: &[Option<MemoryView>], pointer: &[Option<MemoryView>], want: usize) -> bool {
            let present = alias.len().max(pointer.len());
            let unused_empty = |list: &[Option<MemoryView>]| {
                list.iter().skip(want).all(|v| v.as_ref().is_none_or(|v| v.len() == 0))
            };
            (alias.len() >= want || alias.is_empty())
                && (pointer.len() >= want || pointer.is_empty())
                && present >= want
                && unused_empty(alias)
                && unused_empty(pointer)
        }
        if fits(&self.send, &self.pointer_send, send)
            && fits(&self.receive, &self.pointer_receive, receive)
        {
            Ok(())
        } else {
            Err(self.buffer_error())
        }
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
            .flatten()
            .ok_or_else(|| self.buffer_error())
    }
    /// Auto-select input: the map-alias descriptor when it carries data,
    /// otherwise the pointer descriptor. `None` is a genuinely empty buffer.
    pub fn auto_send(&self, index: usize) -> Result<Option<MemoryView>, ServiceError> {
        match (self.send.get(index), self.pointer_send.get(index)) {
            (Some(Some(view)), _) | (_, Some(Some(view))) => Ok(Some(view.clone())),
            (None, None) => Err(self.buffer_error()),
            _ => Ok(None),
        }
    }
    /// Auto-select output, mirroring [`Self::auto_send`].
    pub fn auto_receive(&self, index: usize) -> Result<Option<MemoryView>, ServiceError> {
        match (self.receive.get(index), self.pointer_receive.get(index)) {
            (Some(Some(view)), _) | (_, Some(Some(view))) => Ok(Some(view.clone())),
            (None, None) => Err(self.buffer_error()),
            _ => Ok(None),
        }
    }
    pub fn pointer_send(&self, index: usize) -> Result<Option<MemoryView>, ServiceError> {
        self.pointer_send
            .get(index)
            .cloned()
            .ok_or_else(|| self.buffer_error())
    }
    pub fn pointer_receive(&self, index: usize) -> Result<Option<MemoryView>, ServiceError> {
        self.pointer_receive
            .get(index)
            .cloned()
            .ok_or_else(|| self.buffer_error())
    }
    pub fn receive_buffer(&self, index: usize) -> Result<MemoryView, ServiceError> {
        self.receive
            .get(index)
            .cloned()
            .flatten()
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
    /// Fallible external completion after every kernel preflight, before publication.
    fn complete(&mut self) -> Result<(), ServiceError> {
        Ok(())
    }
    fn commit(self: Box<Self>);
}

pub trait Service: fmt::Debug {
    /// Must isolate mutable request state; do not share mutable internals with
    /// the original. Handle aliases are managed separately by the kernel.
    fn clone_for_request(&self) -> Box<dyn Service>;
    /// Diagnostic name only; never used for dispatch.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError>;
    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(0, 0)?;
        context.require_copy_handles(0)?;
        self.dispatch(request)
    }
    /// Optional conversion into a non-session kernel object (e.g. SharedMemory).
    fn as_handle(&self) -> Option<Object> {
        None
    }
}
