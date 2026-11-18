//! Owned backing and capability-limited views for CPU/GPU aliases. No public
//! borrows or raw pointers escape; the reference executor is single threaded.
use crate::{Access, MemoryError, Permissions};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
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
    /// Live slices (mapped regions and views) by (offset, length), counted.
    slices: RefCell<BTreeMap<(usize, usize), usize>>,
}
impl Backing {
    fn register(&self, offset: usize, len: usize) {
        *self.slices.borrow_mut().entry((offset, len)).or_insert(0) += 1;
    }
}
impl Drop for Backing {
    fn drop(&mut self) {
        self.budget
            .used
            .set(self.budget.used.get() - self.bytes.get_mut().len());
    }
}
#[derive(Debug)]
pub(crate) struct BackingSlice {
    backing: Rc<Backing>,
    offset: usize,
    pub len: usize,
}
impl Clone for BackingSlice {
    fn clone(&self) -> Self {
        self.backing.register(self.offset, self.len);
        Self {
            backing: Rc::clone(&self.backing),
            offset: self.offset,
            len: self.len,
        }
    }
}
impl Drop for BackingSlice {
    fn drop(&mut self) {
        let mut slices = self.backing.slices.borrow_mut();
        let key = (self.offset, self.len);
        let count = slices.get_mut(&key).expect("registered backing slice");
        *count -= 1;
        if *count == 0 {
            slices.remove(&key);
        }
    }
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
        let backing = Rc::new(Backing {
            id: BackingId(id),
            // Discard spare Vec capacity so resident payload accounting
            // matches the backing allocation retained by pinned views.
            bytes: RefCell::new(bytes.into_boxed_slice()),
            budget,
            slices: RefCell::default(),
        });
        backing.register(0, len);
        Ok(Self {
            backing,
            offset: 0,
            len,
        })
    }
    pub fn slice(&self, offset: usize, len: usize) -> Self {
        debug_assert!(offset <= self.len && len <= self.len - offset);
        let offset = self.offset + offset;
        self.backing.register(offset, len);
        Self {
            backing: Rc::clone(&self.backing),
            offset,
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
    /// Live slices of the same backing (regions and views) that overlap
    /// `range`, which must belong to this backing.
    pub(crate) fn overlapping_slices(&self, range: BackingRange) -> usize {
        debug_assert_eq!(range.id, self.backing.id);
        self.backing
            .slices
            .borrow()
            .iter()
            .filter(|&(&(offset, len), _)| offset < range.offset + range.length && range.offset < offset + len)
            .map(|(_, count)| count)
            .sum()
    }
    pub(crate) fn range(&self) -> BackingRange {
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
