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
    Process(ProcessIdentity),
    TransferMemory(Rc<TransferMemory>),
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
    pub fn check_capacity(&self, count: usize) -> Result<(), HandleError> {
        if count > MAX_HANDLES.saturating_sub(self.objects.len())
            || self
                .next
                .checked_add(count as u32)
                .is_none_or(|next| next >= 0x8000_0000)
        {
            return Err(HandleError::Limit);
        }
        Ok(())
    }
    pub fn stage(&self, objects: Vec<Object>) -> Result<Pending, HandleError> {
        self.check_capacity(objects.len())?;
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

/// Opaque host identity; the public PID remains the reference guest PID 1.
#[derive(Clone, Debug)]
pub(crate) struct ProcessIdentity(Rc<()>);
impl ProcessIdentity {
    pub fn new() -> Self {
        Self(Rc::new(()))
    }
    pub fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
#[derive(Debug)]
pub struct TransferMemory {
    owner: ProcessIdentity,
    lease: emulator_memory::MemoryLease,
    quota: Rc<Cell<usize>>,
}
impl TransferMemory {
    pub(crate) fn new(
        owner: ProcessIdentity,
        lease: emulator_memory::MemoryLease,
        quota: Rc<Cell<usize>>,
    ) -> Self {
        quota.set(quota.get() + 1);
        Self {
            owner,
            lease,
            quota,
        }
    }
    pub fn len(&self) -> usize {
        self.lease.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lease.is_empty()
    }
    pub fn owner_permissions(&self) -> emulator_memory::Permissions {
        self.lease.owner_permissions()
    }
    pub fn read(
        &self,
        offset: usize,
        output: &mut [u8],
    ) -> Result<(), emulator_memory::MemoryError> {
        self.lease.read(offset, output)
    }
    pub fn write(&self, offset: usize, input: &[u8]) -> Result<(), emulator_memory::MemoryError> {
        self.lease.write(offset, input)
    }
    pub(crate) fn owned_by(&self, process: &ProcessIdentity) -> bool {
        self.owner.same(process)
    }
}
impl Drop for TransferMemory {
    fn drop(&mut self) {
        self.quota.set(self.quota.get() - 1);
    }
}
