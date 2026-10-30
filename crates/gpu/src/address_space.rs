use crate::{GpuVa, GPU_ADDRESS_LIMIT};
use emulator_memory::{Access, BackingRange, MemoryError, MemoryView};
use std::sync::atomic::AtomicU64;

static NEXT_SPACE_ID: AtomicU64 = AtomicU64::new(1);
pub(crate) const MAX_BACKING_SPANS: usize = 4096;
const MAX_MAPPINGS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressSpaceId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingId {
    space: AddressSpaceId,
    serial: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuMemoryError {
    Empty,
    AddressOverflow,
    Overlap,
    Limit,
    IdentityExhausted,
    InvalidMapping,
    Unmapped(GpuVa),
    PermissionDenied(Access),
    Memory(MemoryError),
}
impl std::fmt::Display for GpuMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GPU memory error: {self:?}")
    }
}
impl std::error::Error for GpuMemoryError {}

#[derive(Debug)]
struct Mapping {
    id: MappingId,
    start: u64,
    end: u64,
    view: MemoryView,
}
#[derive(Clone, Debug)]
struct Part {
    mapping: MappingId,
    view: MemoryView,
}
#[derive(Clone, Debug)]
pub struct GpuMemoryRange {
    parts: Vec<Part>,
    length: usize,
    access: Access,
}
impl GpuMemoryRange {
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn backing_ranges(&self) -> impl Iterator<Item = BackingRange> + '_ {
        self.parts.iter().flat_map(|p| p.view.backing_ranges())
    }
    pub fn mapping_ids(&self) -> impl Iterator<Item = MappingId> + '_ {
        self.parts.iter().map(|p| p.mapping)
    }
    pub fn read(&self, output: &mut [u8]) -> Result<(), GpuMemoryError> {
        if self.access != Access::Read {
            return Err(GpuMemoryError::PermissionDenied(Access::Read));
        }
        if output.len() != self.length {
            return Err(GpuMemoryError::Memory(MemoryError::ViewBounds {
                offset: 0,
                size: output.len(),
                length: self.length,
            }));
        }
        let mut at = 0;
        for part in &self.parts {
            part.view
                .read(0, &mut output[at..at + part.view.len()])
                .map_err(GpuMemoryError::Memory)?;
            at += part.view.len();
        }
        Ok(())
    }
    pub fn write(&self, input: &[u8]) -> Result<(), GpuMemoryError> {
        if self.access != Access::Write {
            return Err(GpuMemoryError::PermissionDenied(Access::Write));
        }
        if input.len() != self.length {
            return Err(GpuMemoryError::Memory(MemoryError::ViewBounds {
                offset: 0,
                size: input.len(),
                length: self.length,
            }));
        }
        let mut at = 0;
        for part in &self.parts {
            part.view
                .write(0, &input[at..at + part.view.len()])
                .map_err(GpuMemoryError::Memory)?;
            at += part.view.len();
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct GpuAddressSpace {
    id: AddressSpaceId,
    mappings: Vec<Mapping>,
    byte_limit: usize,
    next_mapping: u64,
}
impl GpuAddressSpace {
    pub fn new(byte_limit: usize) -> Result<Self, GpuMemoryError> {
        let id =
            crate::allocate_identity(&NEXT_SPACE_ID).ok_or(GpuMemoryError::IdentityExhausted)?;
        Ok(Self {
            id: AddressSpaceId(id),
            mappings: Vec::new(),
            byte_limit,
            next_mapping: 1,
        })
    }
    pub fn id(&self) -> AddressSpaceId {
        self.id
    }
    pub fn mapped_bytes(&self) -> usize {
        self.mappings.iter().map(|m| m.view.len()).sum()
    }
    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
    }
    pub fn map(&mut self, address: GpuVa, view: MemoryView) -> Result<MappingId, GpuMemoryError> {
        let end = range_end(address, view.len())?;
        if self
            .mappings
            .iter()
            .any(|m| m.start < end && address.0 < m.end)
        {
            return Err(GpuMemoryError::Overlap);
        }
        let spans = self
            .mappings
            .iter()
            .map(|m| m.view.backing_ranges().count())
            .sum::<usize>()
            + view.backing_ranges().count();
        if self.mappings.len() >= MAX_MAPPINGS
            || spans > MAX_BACKING_SPANS
            || self
                .mapped_bytes()
                .checked_add(view.len())
                .is_none_or(|n| n > self.byte_limit)
        {
            return Err(GpuMemoryError::Limit);
        }
        let next = self
            .next_mapping
            .checked_add(1)
            .ok_or(GpuMemoryError::IdentityExhausted)?;
        self.mappings
            .try_reserve(1)
            .map_err(|_| GpuMemoryError::Limit)?;
        let id = MappingId {
            space: self.id,
            serial: self.next_mapping,
        };
        self.mappings.push(Mapping {
            id,
            start: address.0,
            end,
            view,
        });
        self.mappings.sort_unstable_by_key(|m| m.start);
        self.next_mapping = next;
        Ok(id)
    }
    pub fn unmap(&mut self, id: MappingId) -> Result<(), GpuMemoryError> {
        let at = self
            .mappings
            .iter()
            .position(|m| m.id == id)
            .ok_or(GpuMemoryError::InvalidMapping)?;
        self.mappings.remove(at);
        Ok(())
    }
    pub fn pin(
        &self,
        address: GpuVa,
        size: usize,
        access: Access,
    ) -> Result<GpuMemoryRange, GpuMemoryError> {
        if access == Access::Execute {
            return Err(GpuMemoryError::PermissionDenied(access));
        }
        let end = range_end(address, size)?;
        let mut cursor = address.0;
        let mut parts = Vec::new();
        for m in &self.mappings {
            if cursor == end {
                break;
            }
            if m.end <= cursor {
                continue;
            }
            if m.start > cursor {
                break;
            }
            if !m.view.permissions().allows(access) {
                return Err(GpuMemoryError::PermissionDenied(access));
            }
            let next = end.min(m.end);
            parts.push(Part {
                mapping: m.id,
                view: m
                    .view
                    .slice((cursor - m.start) as usize, (next - cursor) as usize)
                    .map_err(GpuMemoryError::Memory)?,
            });
            cursor = next;
        }
        if cursor != end {
            return Err(GpuMemoryError::Unmapped(GpuVa(cursor)));
        }
        Ok(GpuMemoryRange {
            parts,
            length: size,
            access,
        })
    }
    pub fn read(&self, address: GpuVa, output: &mut [u8]) -> Result<(), GpuMemoryError> {
        self.pin(address, output.len(), Access::Read)?.read(output)
    }
    pub fn write(&self, address: GpuVa, input: &[u8]) -> Result<(), GpuMemoryError> {
        self.pin(address, input.len(), Access::Write)?.write(input)
    }
}
fn range_end(address: GpuVa, size: usize) -> Result<u64, GpuMemoryError> {
    if size == 0 {
        return Err(GpuMemoryError::Empty);
    }
    address
        .0
        .checked_add(size as u64)
        .filter(|n| *n <= GPU_ADDRESS_LIMIT && address.0 < GPU_ADDRESS_LIMIT)
        .ok_or(GpuMemoryError::AddressOverflow)
}
