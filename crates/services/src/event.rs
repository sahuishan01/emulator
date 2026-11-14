//! Original event handle bridging kernel ReadableEvent to IPC services.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};
use emulator_kernel::Object;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct EventHandle(pub Rc<Cell<bool>>);

impl EventHandle {
    pub fn new(signaled: bool) -> Self {
        Self(Rc::new(Cell::new(signaled)))
    }
}

impl Default for EventHandle {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Service for EventHandle {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, _request: &Request) -> Result<ServiceReply, ServiceError> {
        Ok(ServiceReply::default())
    }
    fn as_handle(&self) -> Option<Object> {
        Some(Object::ReadableEvent(self.0.clone()))
    }
}
