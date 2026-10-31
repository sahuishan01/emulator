//! Scoped lending of original backing. Weak region links restore owner rights
//! automatically when the final lease drops, without a mutable-memory callback.
use crate::{
    fragment, AddressSpace, BackingRange, MemoryError, MemoryView, Permissions, MAX_MAPPINGS,
};
use std::{collections::BTreeMap, rc::Rc};
#[derive(Debug)]
pub(crate) struct LoanState {
    pub permissions: Permissions,
    view: MemoryView,
}
#[derive(Clone, Debug)]
pub struct MemoryLease {
    inner: Rc<LoanState>,
}
impl MemoryLease {
    pub fn len(&self) -> usize {
        self.inner.view.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.view.is_empty()
    }
    pub fn owner_permissions(&self) -> Permissions {
        self.inner.permissions
    }
    pub fn read(&self, offset: usize, bytes: &mut [u8]) -> Result<(), MemoryError> {
        self.inner.view.read(offset, bytes)
    }
    pub fn write(&self, offset: usize, bytes: &[u8]) -> Result<(), MemoryError> {
        self.inner.view.write(offset, bytes)
    }
}
pub(crate) fn overlaps(a: BackingRange, b: BackingRange) -> bool {
    a.id == b.id && a.offset < b.offset + b.length && b.offset < a.offset + a.length
}
impl AddressSpace {
    pub fn lent_bytes(&self) -> usize {
        self.regions
            .iter()
            .filter(|r| r.loaned())
            .map(|r| (r.end - r.start) as usize)
            .sum()
    }
    /// The reference loan requires RW backing with no outstanding views and no
    /// CPU alias of its selected bytes outside this range. Existing capabilities
    /// are never silently revoked. All recoverable failures precede publication.
    pub fn lend(
        &mut self,
        address: u64,
        size: usize,
        owner: Permissions,
    ) -> Result<MemoryLease, MemoryError> {
        if size == 0 {
            return Err(MemoryError::EmptyMapping);
        }
        if ![
            Permissions::NONE,
            Permissions::READ,
            Permissions::READ_WRITE,
        ]
        .contains(&owner)
        {
            return Err(MemoryError::InvalidLoan);
        }
        let end = crate::checked_end(address, size)?;
        let selected = self.covered(address, size, None)?;
        let mut references = BTreeMap::new();
        for r in &self.regions {
            if let Some(b) = &r.bytes {
                *references.entry(b.range().id).or_insert(0_usize) += 1;
            }
        }
        let mut ranges = Vec::new();
        for &(index, offset, length) in &selected {
            let region = &self.regions[index];
            if region.loaned() {
                return Err(MemoryError::LoanConflict);
            }
            let bytes = region
                .bytes
                .as_ref()
                .ok_or(MemoryError::NotCommitted { address })?;
            if region.permissions != Permissions::READ_WRITE {
                return Err(MemoryError::InvalidLoan);
            }
            let backing = bytes.range();
            if bytes.owners() != references[&backing.id] {
                return Err(MemoryError::LoanConflict);
            }
            let selected_range = BackingRange {
                id: backing.id,
                offset: backing.offset + offset,
                length,
            };
            if ranges.iter().any(|&prior| overlaps(prior, selected_range)) {
                return Err(MemoryError::LoanConflict);
            }
            ranges.push(selected_range);
        }
        for a in ranges {
            for region in &self.regions {
                if let Some(bytes) = &region.bytes {
                    let b = bytes.range();
                    if overlaps(a, b) {
                        let low = a.offset.max(b.offset);
                        let high = (a.offset + a.length).min(b.offset + b.length);
                        let alias = region.start + (low - b.offset) as u64;
                        if alias < address || alias + (high - low) as u64 > end {
                            return Err(MemoryError::LoanConflict);
                        }
                    }
                }
            }
        }
        let view = self.pin(address, size, Permissions::READ_WRITE)?;
        let inner = Rc::new(LoanState {
            permissions: owner,
            view,
        });
        let mut result = Vec::new();
        result
            .try_reserve(self.regions.len() + 2)
            .map_err(|_| MemoryError::AllocationFailed)?;
        for r in &self.regions {
            if r.end <= address || r.start >= end {
                result.push(fragment(r, r.start, r.end, r.permissions)?);
                continue;
            }
            if r.start < address {
                result.push(fragment(r, r.start, address, r.permissions)?);
            }
            let mut middle = fragment(r, r.start.max(address), r.end.min(end), r.permissions)?;
            middle.loan = Some(Rc::downgrade(&inner));
            result.push(middle);
            if r.end > end {
                result.push(fragment(r, end, r.end, r.permissions)?);
            }
        }
        if result.len() > MAX_MAPPINGS {
            return Err(MemoryError::TooManyMappings);
        }
        self.regions = result;
        Ok(MemoryLease { inner })
    }
}
