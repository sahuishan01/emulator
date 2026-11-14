//! Guest virtual memory for the initial single-threaded interpreter.

mod loans;
mod shared;
pub use loans::MemoryLease;
use shared::{AllocationBudget, BackingSlice};
pub use shared::{BackingId, BackingRange, MemoryView};
use std::{cell::Cell, fmt, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    read: bool,
    write: bool,
    execute: bool,
}

impl Permissions {
    pub const NONE: Self = Self {
        read: false,
        write: false,
        execute: false,
    };
    pub const READ: Self = Self {
        read: true,
        write: false,
        execute: false,
    };
    pub const WRITE: Self = Self {
        read: false,
        write: true,
        execute: false,
    };
    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        execute: false,
    };
    pub const READ_EXECUTE: Self = Self {
        read: true,
        write: false,
        execute: true,
    };
    pub const EXECUTE: Self = Self {
        read: false,
        write: false,
        execute: true,
    };

    pub fn allows(self, access: Access) -> bool {
        match access {
            Access::Read => self.read,
            Access::Write => self.write,
            Access::Execute => self.execute,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryError {
    EmptyMapping,
    InvalidLoan,
    LoanConflict,
    AddressOverflow,
    OverlappingMapping,
    Unmapped {
        address: u64,
        size: usize,
    },
    PermissionDenied {
        address: u64,
        access: Access,
    },
    UnalignedInstruction {
        address: u64,
    },
    NotReserved {
        address: u64,
    },
    NotCommitted {
        address: u64,
    },
    AllocationLimit,
    AllocationFailed,
    TooManyMappings,
    ViewBounds {
        offset: usize,
        size: usize,
        length: usize,
    },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLoan => write!(f, "unsupported guest memory loan rights or backing"),
            Self::LoanConflict => write!(
                f,
                "guest memory loan conflicts with a loan, alias or outstanding view"
            ),
            Self::EmptyMapping => write!(f, "guest memory mapping must not be empty"),
            Self::AddressOverflow => write!(f, "guest memory address range overflows"),
            Self::OverlappingMapping => write!(f, "guest memory mappings overlap"),
            Self::Unmapped { address, size } => {
                write!(f, "unmapped guest access at {address:#x}, size {size}")
            }
            Self::PermissionDenied { address, access } => {
                write!(f, "guest {access:?} denied at {address:#x}")
            }
            Self::UnalignedInstruction { address } => {
                write!(f, "unaligned instruction address {address:#x}")
            }
            Self::NotReserved { address } => {
                write!(f, "guest range at {address:#x} is not reserved")
            }
            Self::NotCommitted { address } => {
                write!(f, "guest range at {address:#x} is not committed")
            }
            Self::AllocationLimit => write!(f, "guest memory allocation limit exceeded"),
            Self::AllocationFailed => write!(f, "guest memory backing allocation failed"),
            Self::TooManyMappings => write!(f, "guest mapping limit exceeded"),
            Self::ViewBounds {
                offset,
                size,
                length,
            } => write!(f, "backing view range {offset}+{size} exceeds {length}"),
        }
    }
}

impl std::error::Error for MemoryError {}

#[derive(Debug)]
struct Region {
    start: u64,
    end: u64,
    bytes: Option<BackingSlice>,
    permissions: Permissions,
    loan: Option<std::rc::Weak<loans::LoanState>>,
}
impl Region {
    fn loaned(&self) -> bool {
        self.loan
            .as_ref()
            .is_some_and(|loan| loan.strong_count() != 0)
    }
    fn effective_permissions(&self) -> Permissions {
        self.loan
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
            .map_or(self.permissions, |loan| loan.permissions)
    }
}

#[derive(Debug)]
pub struct AddressSpace {
    regions: Vec<Region>,
    budget: Rc<AllocationBudget>,
}

const MAX_MAPPINGS: usize = 4096;

pub struct Mapping {
    pub start: u64,
    pub bytes: Vec<u8>,
    pub permissions: Permissions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingInfo {
    pub start: u64,
    pub end: u64,
    pub permissions: Permissions,
    pub committed: bool,
    pub borrowed: bool,
}

impl Default for AddressSpace {
    fn default() -> Self {
        Self::with_limit(256 * 1024 * 1024)
    }
}

impl AddressSpace {
    pub fn with_limit(byte_limit: usize) -> Self {
        Self {
            regions: Vec::new(),
            budget: Rc::new(AllocationBudget {
                limit: byte_limit,
                used: Cell::new(0),
            }),
        }
    }

    pub fn mapped_bytes(&self) -> usize {
        self.regions
            .iter()
            .filter_map(|region| region.bytes.as_ref())
            .map(|slice| slice.len)
            .sum()
    }

    pub fn query(&self, address: u64) -> Option<MappingInfo> {
        self.regions
            .iter()
            .find(|region| region.start <= address && address < region.end)
            .map(|region| MappingInfo {
                start: region.start,
                end: region.end,
                permissions: region.effective_permissions(),
                borrowed: region.loaned(),
                committed: region.bytes.is_some(),
            })
    }
    /// Originating allocations remain charged while any CPU/GPU view owns them.
    pub fn resident_bytes(&self) -> usize {
        self.budget.used.get()
    }

    pub fn pin(
        &self,
        address: u64,
        size: usize,
        permissions: Permissions,
    ) -> Result<MemoryView, MemoryError> {
        if size == 0 {
            return Err(MemoryError::EmptyMapping);
        }
        let spans = self.covered(address, size, None)?;
        for &(index, _, _) in &spans {
            let region = &self.regions[index];
            if region.bytes.is_none() {
                return Err(MemoryError::NotCommitted { address });
            }
            for access in [Access::Read, Access::Write, Access::Execute] {
                if permissions.allows(access) && !region.effective_permissions().allows(access) {
                    return Err(MemoryError::PermissionDenied {
                        address: region.start.max(address),
                        access,
                    });
                }
            }
        }
        let parts = spans
            .into_iter()
            .map(|(index, offset, size)| {
                self.regions[index]
                    .bytes
                    .as_ref()
                    .expect("committed preflight")
                    .slice(offset, size)
            })
            .collect();
        Ok(MemoryView::new(parts, size, permissions))
    }
    pub fn map_alias(
        &mut self,
        address: u64,
        view: &MemoryView,
        permissions: Permissions,
    ) -> Result<(), MemoryError> {
        if !view.permits(permissions) {
            return Err(MemoryError::PermissionDenied {
                address,
                access: [Access::Read, Access::Write, Access::Execute]
                    .into_iter()
                    .find(|a| permissions.allows(*a) && !view.permissions().allows(*a))
                    .expect("missing permission"),
            });
        }
        checked_end(address, view.len())?;
        let mut next = address;
        let mut pending = Vec::new();
        for span in &view.spans {
            let end = checked_end(next, span.len)?;
            pending.push(Region {
                start: next,
                end,
                bytes: Some(span.clone()),
                permissions,
                loan: None,
            });
            next = end;
        }
        self.append_regions(pending)
    }

    pub fn map(
        &mut self,
        start: u64,
        bytes: Vec<u8>,
        permissions: Permissions,
    ) -> Result<(), MemoryError> {
        self.map_batch(vec![Mapping {
            start,
            bytes,
            permissions,
        }])
    }

    pub fn map_batch(&mut self, mappings: Vec<Mapping>) -> Result<(), MemoryError> {
        if mappings.len() > MAX_MAPPINGS.saturating_sub(self.regions.len()) {
            return Err(MemoryError::TooManyMappings);
        }
        let mut pending = Vec::new();
        for mapping in mappings {
            if mapping.bytes.is_empty() {
                return Err(MemoryError::EmptyMapping);
            }
            let end = checked_end(mapping.start, mapping.bytes.len())?;
            pending.push(Region {
                start: mapping.start,
                end,
                bytes: Some(BackingSlice::new(mapping.bytes, Rc::clone(&self.budget))?),
                permissions: mapping.permissions,
                loan: None,
            });
        }
        self.append_regions(pending)
    }

    fn append_regions(&mut self, pending: Vec<Region>) -> Result<(), MemoryError> {
        for new in &pending {
            if let Some(a) = &new.bytes {
                if self.regions.iter().any(|r| {
                    r.loaned()
                        && r.bytes
                            .as_ref()
                            .is_some_and(|b| loans::overlaps(a.range(), b.range()))
                }) {
                    return Err(MemoryError::LoanConflict);
                }
            }
        }
        if self.regions.len() + pending.len() > MAX_MAPPINGS {
            return Err(MemoryError::TooManyMappings);
        }
        let mut ranges: Vec<_> = self
            .regions
            .iter()
            .chain(&pending)
            .map(|r| (r.start, r.end))
            .collect();
        ranges.sort_unstable();
        if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(MemoryError::OverlappingMapping);
        }
        let additional = pending
            .iter()
            .filter_map(|r| r.bytes.as_ref())
            .try_fold(0_usize, |sum, bytes| sum.checked_add(bytes.len))
            .ok_or(MemoryError::AllocationLimit)?;
        if self
            .mapped_bytes()
            .checked_add(additional)
            .is_none_or(|size| size > self.budget.limit)
        {
            return Err(MemoryError::AllocationLimit);
        }
        self.regions
            .try_reserve(pending.len())
            .map_err(|_| MemoryError::AllocationFailed)?;
        self.regions.extend(pending);
        self.regions.sort_unstable_by_key(|region| region.start);
        Ok(())
    }

    pub fn reserve(&mut self, address: u64, size: usize) -> Result<(), MemoryError> {
        if size == 0 {
            return Err(MemoryError::EmptyMapping);
        }
        let end = checked_end(address, size)?;
        self.append_regions(vec![Region {
            start: address,
            end,
            bytes: None,
            permissions: Permissions::NONE,
            loan: None,
        }])
    }

    pub fn commit(
        &mut self,
        address: u64,
        bytes: Vec<u8>,
        permissions: Permissions,
    ) -> Result<(), MemoryError> {
        if bytes.is_empty() {
            return Err(MemoryError::EmptyMapping);
        }
        let end = checked_end(address, bytes.len())?;
        let spans = self.covered(address, bytes.len(), None)?;
        if spans
            .iter()
            .any(|&(index, _, _)| self.regions[index].bytes.is_some())
        {
            return Err(MemoryError::NotReserved { address });
        }
        if self
            .mapped_bytes()
            .checked_add(bytes.len())
            .is_none_or(|size| size > self.budget.limit)
        {
            return Err(MemoryError::AllocationLimit);
        }
        self.replace_range(
            address,
            end,
            Some(Region {
                start: address,
                end,
                bytes: Some(BackingSlice::new(bytes, Rc::clone(&self.budget))?),
                permissions,
                loan: None,
            }),
            None,
        )
    }

    pub fn protect(
        &mut self,
        address: u64,
        size: usize,
        permissions: Permissions,
    ) -> Result<(), MemoryError> {
        if size == 0 {
            return Err(MemoryError::EmptyMapping);
        }
        let end = checked_end(address, size)?;
        let spans = self.covered(address, size, None)?;
        if spans
            .iter()
            .any(|&(index, _, _)| self.regions[index].bytes.is_none())
        {
            return Err(MemoryError::NotCommitted { address });
        }
        self.replace_range(address, end, None, Some(permissions))
    }

    pub fn unmap(&mut self, address: u64, size: usize) -> Result<(), MemoryError> {
        if size == 0 {
            return Err(MemoryError::EmptyMapping);
        }
        let end = checked_end(address, size)?;
        self.covered(address, size, None)?;
        self.replace_range(address, end, None, None)
    }

    // Build a replacement before publishing it; failed checks or allocations leave mappings intact.
    fn replace_range(
        &mut self,
        start: u64,
        end: u64,
        replacement: Option<Region>,
        protect: Option<Permissions>,
    ) -> Result<(), MemoryError> {
        if self
            .regions
            .iter()
            .any(|r| r.start < end && start < r.end && r.loaned())
        {
            return Err(MemoryError::LoanConflict);
        }
        let mut result = Vec::new();
        result
            .try_reserve(self.regions.len() + 3)
            .map_err(|_| MemoryError::AllocationFailed)?;
        for region in &self.regions {
            if region.end <= start || region.start >= end {
                result.push(fragment(
                    region,
                    region.start,
                    region.end,
                    region.permissions,
                )?);
                continue;
            }
            if region.start < start {
                result.push(fragment(region, region.start, start, region.permissions)?);
            }
            if let Some(permissions) = protect {
                result.push(fragment(
                    region,
                    region.start.max(start),
                    region.end.min(end),
                    permissions,
                )?);
            }
            if region.end > end {
                result.push(fragment(region, end, region.end, region.permissions)?);
            }
        }
        if let Some(region) = replacement {
            result.push(region);
        }
        if result.len() > MAX_MAPPINGS {
            return Err(MemoryError::TooManyMappings);
        }
        let bytes = result
            .iter()
            .filter_map(|r| r.bytes.as_ref())
            .try_fold(0_usize, |sum, bytes| sum.checked_add(bytes.len))
            .ok_or(MemoryError::AllocationLimit)?;
        if bytes > self.budget.limit {
            return Err(MemoryError::AllocationLimit);
        }
        result.sort_unstable_by_key(|r| r.start);
        self.regions = result;
        Ok(())
    }

    fn covered(
        &self,
        address: u64,
        size: usize,
        access: Option<Access>,
    ) -> Result<Vec<(usize, usize, usize)>, MemoryError> {
        let end = checked_end(address, size)?;
        let mut cursor = address;
        let mut spans = Vec::new();
        for (index, region) in self.regions.iter().enumerate() {
            if cursor == end {
                break;
            }
            if region.end <= cursor {
                continue;
            }
            if region.start > cursor {
                break;
            }
            if let Some(access) = access {
                if !region.effective_permissions().allows(access) {
                    return Err(MemoryError::PermissionDenied {
                        address: cursor,
                        access,
                    });
                }
            }
            let next = end.min(region.end);
            spans.push((
                index,
                (cursor - region.start) as usize,
                (next - cursor) as usize,
            ));
            cursor = next;
        }
        if cursor != end {
            return Err(MemoryError::Unmapped {
                address: cursor,
                size: (end - cursor) as usize,
            });
        }
        Ok(spans)
    }

    pub fn check_access(
        &self,
        address: u64,
        size: usize,
        access: Access,
    ) -> Result<(), MemoryError> {
        if size == 0 {
            return Ok(());
        }
        self.covered(address, size, Some(access)).map(|_| ())
    }

    pub fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        if output.is_empty() {
            return Ok(());
        }
        self.read_access(address, output, Access::Read)
    }

    fn read_access(
        &self,
        address: u64,
        output: &mut [u8],
        access: Access,
    ) -> Result<(), MemoryError> {
        let spans = self.covered(address, output.len(), Some(access))?;
        let mut written = 0;
        for (index, offset, size) in spans {
            let bytes = self.regions[index]
                .bytes
                .as_ref()
                .expect("accessible region has backing");
            bytes.read(offset, &mut output[written..written + size]);
            written += size;
        }
        Ok(())
    }

    pub fn write(&mut self, address: u64, input: &[u8]) -> Result<(), MemoryError> {
        if input.is_empty() {
            return Ok(());
        }
        let spans = self.covered(address, input.len(), Some(Access::Write))?;
        let mut consumed = 0;
        for (index, offset, size) in spans {
            let bytes = self.regions[index]
                .bytes
                .as_ref()
                .expect("writable region has backing");
            bytes.write(offset, &input[consumed..consumed + size]);
            consumed += size;
        }
        Ok(())
    }

    pub fn fetch_u32(&self, address: u64) -> Result<u32, MemoryError> {
        if address & 3 != 0 {
            return Err(MemoryError::UnalignedInstruction { address });
        }
        let mut bytes = [0; 4];
        self.read_access(address, &mut bytes, Access::Execute)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn read_u8(&self, address: u64) -> Result<u8, MemoryError> {
        let mut bytes = [0; 1];
        self.read(address, &mut bytes)?;
        Ok(bytes[0])
    }

    pub fn read_u16(&self, address: u64) -> Result<u16, MemoryError> {
        let mut bytes = [0; 2];
        self.read(address, &mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn read_u32(&self, address: u64) -> Result<u32, MemoryError> {
        let mut bytes = [0; 4];
        self.read(address, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn read_u64(&self, address: u64) -> Result<u64, MemoryError> {
        let mut bytes = [0; 8];
        self.read(address, &mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn write_u8(&mut self, address: u64, value: u8) -> Result<(), MemoryError> {
        self.write(address, &[value])
    }

    pub fn write_u16(&mut self, address: u64, value: u16) -> Result<(), MemoryError> {
        self.write(address, &value.to_le_bytes())
    }

    pub fn write_u32(&mut self, address: u64, value: u32) -> Result<(), MemoryError> {
        self.write(address, &value.to_le_bytes())
    }

    pub fn write_u64(&mut self, address: u64, value: u64) -> Result<(), MemoryError> {
        self.write(address, &value.to_le_bytes())
    }
}

fn fragment(
    region: &Region,
    start: u64,
    end: u64,
    permissions: Permissions,
) -> Result<Region, MemoryError> {
    let bytes = if let Some(bytes) = &region.bytes {
        let size = (end - start) as usize;
        let offset = (start - region.start) as usize;
        Some(bytes.slice(offset, size))
    } else {
        None
    };
    Ok(Region {
        start,
        end,
        bytes,
        permissions,
        loan: region.loan.clone(),
    })
}

fn checked_end(start: u64, size: usize) -> Result<u64, MemoryError> {
    start
        .checked_add(u64::try_from(size).map_err(|_| MemoryError::AddressOverflow)?)
        .ok_or(MemoryError::AddressOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn little_endian_unaligned_data_access() {
        let mut memory = AddressSpace::default();
        memory
            .map(0x1000, vec![0; 16], Permissions::READ_WRITE)
            .unwrap();
        memory.write_u64(0x1001, 0x0102_0304_0506_0708).unwrap();
        assert_eq!(memory.read_u32(0x1001).unwrap(), 0x0506_0708);
        assert_eq!(memory.read_u64(0x1001).unwrap(), 0x0102_0304_0506_0708);
    }

    #[test]
    fn mapping_checks_are_not_host_pointer_arithmetic() {
        let mut memory = AddressSpace::default();
        assert_eq!(
            memory.map(0, vec![], Permissions::READ),
            Err(MemoryError::EmptyMapping)
        );
        assert_eq!(
            memory.map(u64::MAX, vec![0], Permissions::READ),
            Err(MemoryError::AddressOverflow)
        );
        memory
            .map(0x1000, vec![0; 8], Permissions::READ_WRITE)
            .unwrap();
        assert_eq!(
            memory.map(0x1007, vec![0; 8], Permissions::READ),
            Err(MemoryError::OverlappingMapping)
        );
        memory.map(0x1008, vec![0; 8], Permissions::READ).unwrap();
        assert!(matches!(
            memory.read_u64(u64::MAX),
            Err(MemoryError::AddressOverflow)
        ));
    }

    #[test]
    fn failed_write_does_not_partially_modify_guest_memory() {
        let mut memory = AddressSpace::default();
        memory
            .map(0x1000, vec![0xaa; 8], Permissions::READ_WRITE)
            .unwrap();
        assert!(memory.write_u64(0x1004, 0).is_err());
        assert_eq!(memory.read_u64(0x1000).unwrap(), 0xaaaa_aaaa_aaaa_aaaa);
    }

    #[test]
    fn read_write_execute_are_independent() {
        let mut memory = AddressSpace::default();
        memory
            .map(0x1000, vec![0; 8], Permissions::READ_WRITE)
            .unwrap();
        memory
            .map(0x2000, vec![0; 8], Permissions::EXECUTE)
            .unwrap();
        assert!(matches!(
            memory.fetch_u32(0x1000),
            Err(MemoryError::PermissionDenied {
                access: Access::Execute,
                ..
            })
        ));
        assert_eq!(memory.fetch_u32(0x2000).unwrap(), 0);
        assert!(matches!(
            memory.read_u32(0x2000),
            Err(MemoryError::PermissionDenied {
                access: Access::Read,
                ..
            })
        ));
        assert!(matches!(
            memory.write_u32(0x2000, 1),
            Err(MemoryError::PermissionDenied {
                access: Access::Write,
                ..
            })
        ));
        assert!(matches!(
            memory.fetch_u32(0x2001),
            Err(MemoryError::UnalignedInstruction { .. })
        ));
    }
}
