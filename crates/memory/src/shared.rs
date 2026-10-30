//! Owned backing and capability-limited views for CPU/GPU aliases. No public
//! borrows or raw pointers escape; the reference executor is single threaded.
use crate::{Access, MemoryError, Permissions};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_BACKING_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackingId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackingRange {
    pub id: BackingId,
    pub offset: usize,
    pub length: usize,
}

#[derive(Debug)]
pub(crate) struct AllocationBudget {
    pub limit: usize,
    pub used: Cell<usize>,
}
#[derive(Debug)]
struct Backing {
    id: BackingId,
    bytes: RefCell<Box<[u8]>>,
    budget: Rc<AllocationBudget>,
}
impl Drop for Backing {
    fn drop(&mut self) {
        self.budget
            .used
            .set(self.budget.used.get() - self.bytes.get_mut().len());
    }
}
#[derive(Clone, Debug)]
pub(crate) struct BackingSlice {
    backing: Rc<Backing>,
    offset: usize,
    pub len: usize,
}
impl BackingSlice {
    pub fn new(bytes: Vec<u8>, budget: Rc<AllocationBudget>) -> Result<Self, MemoryError> {
        let used = budget
            .used
            .get()
            .checked_add(bytes.len())
            .filter(|n| *n <= budget.limit)
            .ok_or(MemoryError::AllocationLimit)?;
        let mut id = NEXT_BACKING_ID.load(Ordering::Relaxed);
        loop {
            let next = id.checked_add(1).ok_or(MemoryError::AllocationLimit)?;
            match NEXT_BACKING_ID.compare_exchange_weak(
                id,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => id = actual,
            }
        }
        let len = bytes.len();
        budget.used.set(used);
        Ok(Self {
            backing: Rc::new(Backing {
                id: BackingId(id),
                // Discard spare Vec capacity so resident payload accounting
                // matches the backing allocation retained by pinned views.
                bytes: RefCell::new(bytes.into_boxed_slice()),
                budget,
            }),
            offset: 0,
            len,
        })
    }
    pub fn slice(&self, offset: usize, len: usize) -> Self {
        debug_assert!(offset <= self.len && len <= self.len - offset);
        Self {
            backing: Rc::clone(&self.backing),
            offset: self.offset + offset,
            len,
        }
    }
    pub fn read(&self, offset: usize, output: &mut [u8]) {
        let start = self.offset + offset;
        output.copy_from_slice(&self.backing.bytes.borrow()[start..start + output.len()]);
    }
    pub fn write(&self, offset: usize, input: &[u8]) {
        let start = self.offset + offset;
        self.backing.bytes.borrow_mut()[start..start + input.len()].copy_from_slice(input);
    }
    fn range(&self) -> BackingRange {
        BackingRange {
            id: self.backing.id,
            offset: self.offset,
            length: self.len,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemoryView {
    pub(crate) spans: Vec<BackingSlice>,
    length: usize,
    permissions: Permissions,
}
impl MemoryView {
    pub(crate) fn new(spans: Vec<BackingSlice>, length: usize, permissions: Permissions) -> Self {
        Self {
            spans,
            length,
            permissions,
        }
    }
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn permissions(&self) -> Permissions {
        self.permissions
    }
    pub fn backing_ranges(&self) -> impl Iterator<Item = BackingRange> + '_ {
        self.spans.iter().map(BackingSlice::range)
    }
    fn bounds(&self, offset: usize, size: usize) -> Result<(), MemoryError> {
        if offset > self.length || size > self.length - offset {
            return Err(MemoryError::ViewBounds {
                offset,
                size,
                length: self.length,
            });
        }
        Ok(())
    }
    pub(crate) fn permits(&self, permissions: Permissions) -> bool {
        [Access::Read, Access::Write, Access::Execute]
            .into_iter()
            .all(|a| !permissions.allows(a) || self.permissions.allows(a))
    }
    pub fn slice(&self, offset: usize, length: usize) -> Result<Self, MemoryError> {
        self.bounds(offset, length)?;
        if length == 0 {
            return Err(MemoryError::EmptyMapping);
        }
        let mut spans = Vec::new();
        spans
            .try_reserve(self.spans.len())
            .map_err(|_| MemoryError::AllocationFailed)?;
        let mut skip = offset;
        let mut left = length;
        for span in &self.spans {
            if skip >= span.len {
                skip -= span.len;
                continue;
            }
            let count = left.min(span.len - skip);
            spans.push(span.slice(skip, count));
            left -= count;
            skip = 0;
            if left == 0 {
                break;
            }
        }
        Ok(Self::new(spans, length, self.permissions))
    }
    pub fn read(&self, offset: usize, output: &mut [u8]) -> Result<(), MemoryError> {
        self.bounds(offset, output.len())?;
        if !self.permissions.allows(Access::Read) {
            return Err(MemoryError::PermissionDenied {
                address: offset as u64,
                access: Access::Read,
            });
        }
        let mut skip = offset;
        let mut copied = 0;
        for span in &self.spans {
            if copied == output.len() {
                break;
            }
            if skip >= span.len {
                skip -= span.len;
                continue;
            }
            let count = (output.len() - copied).min(span.len - skip);
            span.read(skip, &mut output[copied..copied + count]);
            copied += count;
            skip = 0;
        }
        Ok(())
    }
    pub fn write(&self, offset: usize, input: &[u8]) -> Result<(), MemoryError> {
        self.bounds(offset, input.len())?;
        if !self.permissions.allows(Access::Write) {
            return Err(MemoryError::PermissionDenied {
                address: offset as u64,
                access: Access::Write,
            });
        }
        let mut skip = offset;
        let mut copied = 0;
        for span in &self.spans {
            if copied == input.len() {
                break;
            }
            if skip >= span.len {
                skip -= span.len;
                continue;
            }
            let count = (input.len() - copied).min(span.len - skip);
            span.write(skip, &input[copied..copied + count]);
            copied += count;
            skip = 0;
        }
        Ok(())
    }
}
