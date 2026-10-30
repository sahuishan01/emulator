//! Host service contract. Preparing on an isolated clone allows the kernel to
//! validate/encode output and stage handles before publishing service state.
use emulator_ipc::Request;
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
}

pub trait Service: fmt::Debug {
    /// Must isolate mutable request state; do not share mutable internals with
    /// the original. Handle aliases are managed separately by the kernel.
    fn clone_for_request(&self) -> Box<dyn Service>;
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError>;
}
