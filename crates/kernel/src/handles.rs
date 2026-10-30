use crate::service::Service;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct ThreadObject {
    pub id: usize,
    pub terminated: Rc<Cell<bool>>,
}
impl ThreadObject {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            terminated: Rc::new(Cell::new(false)),
        }
    }
}

pub const MAX_HANDLES: usize = 256;
pub const MAIN_THREAD_HANDLE: u32 = 0x100;

#[derive(Clone, Debug)]
pub enum Object {
    Thread(ThreadObject),
    Session(Rc<RefCell<Box<dyn Service>>>),
    WritableEvent(Rc<Cell<bool>>),
    ReadableEvent(Rc<Cell<bool>>),
}
impl Object {
    pub fn session(service: Box<dyn Service>) -> Self {
        Self::Session(Rc::new(RefCell::new(service)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleError {
    Invalid(u32),
    Limit,
}

#[derive(Debug)]
pub struct Pending {
    pub ids: Vec<u32>,
    objects: Vec<Object>,
    next: u32,
}

#[derive(Debug)]
pub struct HandleTable {
    objects: BTreeMap<u32, Object>,
    next: u32,
}
impl HandleTable {
    pub fn new(main: ThreadObject) -> Self {
        Self {
            objects: BTreeMap::from([(MAIN_THREAD_HANDLE, Object::Thread(main))]),
            next: MAIN_THREAD_HANDLE + 1,
        }
    }
    pub fn get(&self, handle: u32) -> Result<Object, HandleError> {
        self.objects
            .get(&handle)
            .cloned()
            .ok_or(HandleError::Invalid(handle))
    }
    pub fn close(&mut self, handle: u32) -> Result<(), HandleError> {
        self.objects
            .remove(&handle)
            .map(|_| ())
            .ok_or(HandleError::Invalid(handle))
    }
    pub fn stage(&self, objects: Vec<Object>) -> Result<Pending, HandleError> {
        if self.objects.len() + objects.len() > MAX_HANDLES {
            return Err(HandleError::Limit);
        }
        let next = self
            .next
            .checked_add(objects.len() as u32)
            .filter(|n| *n < 0x8000_0000)
            .ok_or(HandleError::Limit)?;
        Ok(Pending {
            ids: (self.next..next).collect(),
            objects,
            next,
        })
    }
    pub fn commit(&mut self, pending: Pending) {
        for (id, object) in pending.ids.into_iter().zip(pending.objects) {
            self.objects.insert(id, object);
        }
        self.next = pending.next;
    }
    pub fn len(&self) -> usize {
        self.objects.len()
    }
    pub fn clear(&mut self) {
        self.objects.clear();
    }
}
