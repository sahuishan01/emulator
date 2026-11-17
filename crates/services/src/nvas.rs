//! Original bounded NV GPU address spaces backed by the shared GPU memory core.
use crate::nvmap::{ImportBudget, BAD_PARAMETER, IMPORT_BYTES, NOT_SUPPORTED, NO_MEMORY};
use emulator_gpu::{GpuAddressSpace, GpuMemoryError, GpuVa, MappingId};
use emulator_memory::MemoryView;

pub(crate) const BIND_CHANNEL: u32 = 0x40044101;
pub(crate) const INITIALIZE: u32 = 0x40284109;
pub(crate) const MAP_BUFFER: u32 = 0xc0284106;
pub(crate) const UNMAP_BUFFER: u32 = 0xc0084105;
/// `_IOWR('A', 2, nvgpu_as_alloc_space_args)`: {pages u32, page_size u32,
/// flags u32, pad u32, offset/align u64}.
pub(crate) const ALLOC_SPACE: u32 = 0xc0184102;
/// `_IOWR('A', 3, nvgpu_as_free_space_args)`: {offset u64, pages u32, page_size u32}.
pub(crate) const FREE_SPACE: u32 = 0xc0104103;
const ALLOC_FIXED_OFFSET: u32 = 1;
const MAP_FIXED_OFFSET: u32 = 1;
const MAP_CACHEABLE: u32 = 4;
const ALLOC_SPARSE: u32 = 2;
const MAX_RESERVATIONS: usize = 256;
/// `_IOWR('A', 8, nvgpu_as_get_va_regions_args)`: {buf_addr u64, buf_size
/// u32, reserved u32} then two inline `{offset u64, page_size u32, pad, pages u64}`.
pub(crate) const GET_VA_REGIONS: u32 = 0xc0404108;
const VA_BASE: u64 = 1 << 32;
const VA_END: u64 = 1 << 40;
#[derive(Clone, Debug)]
struct Region {
    address: u64,
    length: usize,
    id: MappingId,
    handle: u32,
}
#[derive(Debug, Default)]
pub(crate) struct NvAddressSpace {
    space: Option<GpuAddressSpace>,
    regions: Vec<Region>,
    /// Allocation window [start, end); `None` uses `VA_BASE..VA_END`.
    window: Option<(u64, u64)>,
    /// Small/big page boundary inside `window`.
    split: Option<u64>,
    /// VA reservations from ALLOC_SPACE as sorted (address, length). Sparse
    /// reservations are tracked like plain ones: GPU faults are not modelled.
    reserved: Vec<(u64, u64)>,
}
impl Clone for NvAddressSpace {
    fn clone(&self) -> Self {
        Self {
            space: self
                .space
                .as_ref()
                .map(GpuAddressSpace::snapshot_for_transaction),
            regions: self.regions.clone(),
            window: self.window,
            split: self.split,
            reserved: self.reserved.clone(),
        }
    }
}
fn word(input: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(input[at..at + 4].try_into().expect("validated NV AS word"))
}
fn wide(input: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(
        input[at..at + 8]
            .try_into()
            .expect("validated NV AS wide word"),
    )
}
fn memory_error(error: GpuMemoryError) -> u32 {
    match error {
        GpuMemoryError::Limit | GpuMemoryError::IdentityExhausted => NO_MEMORY,
        _ => BAD_PARAMETER,
    }
}
/// Mapped regions plus reservations as sorted (address, length).
fn occupied(regions: &[Region], reserved: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut all: Vec<(u64, u64)> = regions
        .iter()
        .map(|r| (r.address, r.length as u64))
        .chain(reserved.iter().copied())
        .collect();
    all.sort_unstable();
    all
}
/// First `align`-aligned gap of `length` bytes inside `window`.
fn free_range(occupied: Vec<(u64, u64)>, window: (u64, u64), length: u64, align: u64) -> Result<u64, u32> {
    let aligned = |value: u64| {
        value
            .checked_add(align - 1)
            .map(|v| v & !(align - 1))
            .ok_or(NO_MEMORY)
    };
    let mut address = aligned(window.0)?;
    for (start, len) in occupied {
        if address.checked_add(length).ok_or(NO_MEMORY)? <= start {
            break;
        }
        if address < start + len {
            address = aligned(start + len)?;
        }
    }
    if address.checked_add(length).ok_or(NO_MEMORY)? > window.1 {
        return Err(NO_MEMORY);
    }
    Ok(address)
}
impl NvAddressSpace {
    pub fn argument_size(code: u32) -> Option<usize> {
        match code {
            BIND_CHANNEL => Some(4),
            INITIALIZE | MAP_BUFFER => Some(40),
            UNMAP_BUFFER => Some(8),
            ALLOC_SPACE => Some(0x18),
            FREE_SPACE => Some(0x10),
            GET_VA_REGIONS => Some(0x40),
            _ => None,
        }
    }
    pub fn usage(&self) -> (usize, usize) {
        self.space
            .as_ref()
            .map_or((0, 0), |s| (s.mapped_bytes(), s.mapped_spans()))
    }
    pub fn core(&self) -> Result<&GpuAddressSpace, u32> {
        self.space.as_ref().ok_or(3)
    }
    pub fn holds(&self, handle: u32) -> bool {
        self.regions.iter().any(|r| r.handle == handle)
    }
    pub fn ioctl(
        &mut self,
        code: u32,
        input: &[u8],
        budget: ImportBudget,
        lookup: impl FnOnce(u32) -> Result<MemoryView, u32>,
    ) -> Result<Vec<u8>, u32> {
        if Some(input.len()) != Self::argument_size(code) {
            return Err(BAD_PARAMETER);
        }
        let mut output = input.to_vec();
        if code == INITIALIZE {
            if self.space.is_some() {
                return Err(8);
            }
            // Public interface descriptions disagree on the first/third word
            // order. Accept their default flags (0/1) and big page (0/64K)
            // combinations explicitly; only 4K buffer mappings are implemented.
            let first = word(input, 0);
            let third = word(input, 8);
            let client_order = first <= 1 && [0, 65536].contains(&third);
            let alternate_order = [0, 65536].contains(&first) && third <= 1;
            if !(client_order || alternate_order) {
                return Err(NOT_SUPPORTED);
            }
            if word(input, 4) != 0 || word(input, 12) != 0 {
                return Err(BAD_PARAMETER);
            }
            // Optional explicit VA range {start, end, split}; all zero keeps
            // the default window. Buffers are 4 KiB-mapped anywhere in it.
            let (start, end, split) = (wide(input, 16), wide(input, 24), wide(input, 32));
            let window = if (start, end, split) == (0, 0, 0) {
                None
            } else {
                let page_aligned = [start, end, split].iter().all(|v| v % 4096 == 0);
                if !page_aligned || start == 0 || !(start <= split && split <= end) || end > VA_END {
                    return Err(BAD_PARAMETER);
                }
                Some((start, end, split))
            };
            self.space = Some(GpuAddressSpace::new(IMPORT_BYTES).map_err(memory_error)?);
            self.window = window.map(|(start, end, _)| (start, end));
            self.split = window.map(|(_, _, split)| split);
            return Ok(output);
        }
        let (va_base, va_end) = self.window.unwrap_or((VA_BASE, VA_END));
        let split = self.split.unwrap_or(va_end);
        let space = self.space.as_mut().ok_or(3_u32)?;
        match code {
            ALLOC_SPACE => {
                let (pages, page_size, flags) = (word(input, 0), word(input, 4), word(input, 8));
                if flags & !(ALLOC_FIXED_OFFSET | ALLOC_SPARSE) != 0 || word(input, 12) != 0 {
                    return Err(NOT_SUPPORTED);
                }
                if ![4096, 65536, 131072].contains(&page_size) || pages == 0 {
                    return Err(BAD_PARAMETER);
                }
                let length = u64::from(pages) * u64::from(page_size);
                if self.reserved.len() >= MAX_RESERVATIONS {
                    return Err(NO_MEMORY);
                }
                let occupied = occupied(&self.regions, &self.reserved);
                let address = if flags & ALLOC_FIXED_OFFSET != 0 {
                    let at = wide(input, 16);
                    let end = at.checked_add(length).ok_or(BAD_PARAMETER)?;
                    if at % u64::from(page_size) != 0 || at < va_base || end > va_end {
                        return Err(BAD_PARAMETER);
                    }
                    if occupied.iter().any(|&(a, l)| at < a + l && a < end) {
                        return Err(BAD_PARAMETER);
                    }
                    at
                } else {
                    let align = wide(input, 16).max(u64::from(page_size));
                    if !align.is_power_of_two() {
                        return Err(BAD_PARAMETER);
                    }
                    free_range(occupied, (va_base, va_end), length, align)?
                };
                self.reserved.try_reserve(1).map_err(|_| NO_MEMORY)?;
                self.reserved.push((address, length));
                self.reserved.sort_unstable();
                output[16..24].copy_from_slice(&address.to_le_bytes());
            }
            FREE_SPACE => {
                let (at, pages, page_size) = (wide(input, 0), word(input, 8), word(input, 12));
                let length = u64::from(pages) * u64::from(page_size);
                let index = self
                    .reserved
                    .iter()
                    .position(|&r| r == (at, length))
                    .ok_or(BAD_PARAMETER)?;
                // Mappings inside must be unmapped first; nothing is implicit.
                if self.regions.iter().any(|r| r.address < at + length && at < r.address + r.length as u64) {
                    return Err(BAD_PARAMETER);
                }
                self.reserved.remove(index);
            }
            GET_VA_REGIONS => {
                // Small-page region below the split, big-page region above.
                output[8..12].copy_from_slice(&48u32.to_le_bytes());
                let regions = [(va_base, 4096u32, split - va_base), (split, 65536, va_end - split)];
                for (i, (offset, page, bytes)) in regions.into_iter().enumerate() {
                    let at = 16 + i * 24;
                    output[at..at + 8].copy_from_slice(&offset.to_le_bytes());
                    output[at + 8..at + 16].copy_from_slice(&u64::from(page).to_le_bytes());
                    output[at + 16..at + 24].copy_from_slice(&(bytes / u64::from(page)).to_le_bytes());
                }
            }
            MAP_BUFFER => {
                let page_size = word(input, 12);
                let flags = word(input, 0);
                // FIXED_OFFSET (1) and CACHEABLE (4); cacheability has no
                // observable effect on the linear GPU memory model.
                if flags & !(MAP_FIXED_OFFSET | MAP_CACHEABLE) != 0
                    // PTE kind (0xff, or -1 for "use the nvmap kind"): the GPU
                    // model addresses memory linearly, so kinds are accepted
                    // without compression/tiling side effects.
                    || !(word(input, 4) <= 0xff || word(input, 4) == u32::MAX)
                    || ![0, 4096, 65536, 131072].contains(&page_size)
                {
                    return Err(NOT_SUPPORTED);
                }
                let handle = word(input, 8);
                let view = lookup(handle)?;
                let offset = usize::try_from(wide(input, 16)).map_err(|_| BAD_PARAMETER)?;
                let requested = usize::try_from(wide(input, 24)).map_err(|_| BAD_PARAMETER)?;
                if offset % 4096 != 0 || offset >= view.len() {
                    return Err(BAD_PARAMETER);
                }
                let length = if requested == 0 {
                    view.len() - offset
                } else {
                    requested
                };
                if length == 0 || length % 4096 != 0 || length > view.len() - offset {
                    return Err(BAD_PARAMETER);
                }
                // The last field is the alignment, or the GPU VA when fixed.
                let align = if flags & MAP_FIXED_OFFSET != 0 { 0 } else { wide(input, 32) };
                if align != 0 && (!align.is_power_of_two() || align > IMPORT_BYTES as u64) {
                    return Err(BAD_PARAMETER);
                }
                // Big-page mappings are VA-aligned to the big page and live in
                // the big-page region when an explicit split was configured;
                // backing is still mapped at 4 KiB granularity.
                let page_size = u64::from(page_size).max(4096);
                let align = align.max(page_size);
                let map_window = match self.split {
                    Some(split) if page_size > 4096 => (split, va_end),
                    Some(split) => (va_base, split),
                    None => (va_base, va_end),
                };
                if self.regions.len() >= 64 || length > budget.bytes {
                    return Err(NO_MEMORY);
                }
                let view = view.slice(offset, length).map_err(|_| BAD_PARAMETER)?;
                if view.backing_ranges().count() > budget.spans {
                    return Err(NO_MEMORY);
                }
                let address = if flags & MAP_FIXED_OFFSET != 0 {
                    // Fixed maps may land inside reservations, never on top of
                    // another mapping.
                    let at = wide(input, 32);
                    let end = at.checked_add(length as u64).ok_or(BAD_PARAMETER)?;
                    if at % page_size != 0 || at < va_base || end > va_end {
                        return Err(BAD_PARAMETER);
                    }
                    if self
                        .regions
                        .iter()
                        .any(|r| at < r.address + r.length as u64 && r.address < end)
                    {
                        return Err(BAD_PARAMETER);
                    }
                    at
                } else {
                    free_range(
                        occupied(&self.regions, &self.reserved),
                        map_window,
                        length as u64,
                        align,
                    )?
                };
                self.regions.try_reserve(1).map_err(|_| NO_MEMORY)?;
                let id = space.map(GpuVa(address), view).map_err(memory_error)?;
                self.regions.push(Region {
                    address,
                    length,
                    id,
                    handle,
                });
                self.regions.sort_unstable_by_key(|r| r.address);
                output[4..8].fill(0);
                output[12..16].copy_from_slice(&(page_size as u32).to_le_bytes());
                output[32..40].copy_from_slice(&address.to_le_bytes());
            }
            UNMAP_BUFFER => {
                let at = self
                    .regions
                    .iter()
                    .position(|r| r.address == wide(input, 0))
                    .ok_or(BAD_PARAMETER)?;
                space.unmap(self.regions[at].id).map_err(memory_error)?;
                self.regions.remove(at);
            }
            _ => return Err(NOT_SUPPORTED),
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emulator_memory::{AddressSpace, Permissions};
    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
    fn budget() -> ImportBudget {
        ImportBudget {
            bytes: IMPORT_BYTES,
            spans: 4096,
        }
    }
    fn init() -> NvAddressSpace {
        let mut a = NvAddressSpace::default();
        let mut args = [0; 40];
        args[..4].copy_from_slice(&1_u32.to_le_bytes());
        a.ioctl(INITIALIZE, &args, budget(), |_| Err(4)).unwrap();
        a
    }
    fn map(a: &mut NvAddressSpace, view: &MemoryView, args: &[u32]) -> Result<Vec<u8>, u32> {
        a.ioctl(MAP_BUFFER, &bytes(args), budget(), |h| {
            if h == 1 {
                Ok(view.clone())
            } else {
                Err(4)
            }
        })
    }
    #[test]
    fn mapped_slices_preserve_rights_and_original_bytes_across_cpu_unmap_and_reuse() {
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![0; 8192], Permissions::READ_WRITE)
            .unwrap();
        m.write_u64(0x9000, 42).unwrap();
        let view = m.pin(0x8000, 8192, Permissions::READ_WRITE).unwrap();
        let mut a = init();
        let out = map(&mut a, &view, &[0, u32::MAX, 1, 0, 4096, 0, 4096, 0, 0, 0]).unwrap();
        assert_eq!(wide(&out, 32), VA_BASE);
        assert_eq!(word(&out, 4), 0);
        assert_eq!(word(&out, 12), 4096);
        let mut actual = [0; 8];
        a.space
            .as_ref()
            .unwrap()
            .read(GpuVa(VA_BASE), &mut actual)
            .unwrap();
        assert_eq!(u64::from_le_bytes(actual), 42);
        a.space
            .as_ref()
            .unwrap()
            .write(GpuVa(VA_BASE), &77_u64.to_le_bytes())
            .unwrap();
        assert_eq!(m.read_u64(0x9000).unwrap(), 77);
        drop(view);
        m.unmap(0x8000, 8192).unwrap();
        assert_eq!(m.resident_bytes(), 8192);
        m.map(0x8000, vec![99; 8192], Permissions::READ_WRITE)
            .unwrap();
        a.space
            .as_ref()
            .unwrap()
            .read(GpuVa(VA_BASE), &mut actual)
            .unwrap();
        assert_eq!(u64::from_le_bytes(actual), 77);
        a.space
            .as_ref()
            .unwrap()
            .write(GpuVa(VA_BASE), &88_u64.to_le_bytes())
            .unwrap();
        assert_eq!(m.read_u64(0x9000).unwrap(), 0x6363636363636363);
        a.ioctl(UNMAP_BUFFER, &VA_BASE.to_le_bytes(), budget(), |_| Err(4))
            .unwrap();
        assert_eq!(a.usage(), (0, 0));
        assert_eq!(m.resident_bytes(), 8192);
        let readonly = m.pin(0x8000, 4096, Permissions::READ).unwrap();
        map(&mut a, &readonly, &[4, 0, 1, 4096, 0, 0, 0, 0, 0, 0]).unwrap();
        assert!(a
            .space
            .as_ref()
            .unwrap()
            .write(GpuVa(VA_BASE), &[1])
            .is_err());
    }
    #[test]
    fn unsupported_and_invalid_requests_do_not_publish_regions_or_change_existing_maps() {
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![0; 8192], Permissions::READ_WRITE)
            .unwrap();
        let view = m.pin(0x8000, 8192, Permissions::READ_WRITE).unwrap();
        let mut a = init();
        for (args, error) in [
            ([2, 0, 1, 4096, 0, 0, 4096, 0, 0, 0], 2),
            ([0, 0x100, 1, 4096, 0, 0, 4096, 0, 0, 0], 2),
            ([0, 0, 1, 8192, 0, 0, 4096, 0, 0, 0], 2),
            ([0, 0, 99, 4096, 0, 0, 4096, 0, 0, 0], 4),
            ([0, 0, 1, 4096, 1, 0, 4096, 0, 0, 0], 4),
            ([0, 0, 1, 4096, 8192, 0, 4096, 0, 0, 0], 4),
            ([0, 0, 1, 4096, 0, 0, 12288, 0, 0, 0], 4),
            ([0, 0, 1, 4096, 0, 0, 4097, 0, 0, 0], 4),
            ([0, 0, 1, 4096, 0, 0, 4096, 0, 3, 0], 4),
        ] {
            assert_eq!(map(&mut a, &view, &args), Err(error));
            assert_eq!(a.usage(), (0, 0));
        }
        map(&mut a, &view, &[0, 0, 1, 0, 4096, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(a.usage(), (4096, 1));
        assert_eq!(
            a.ioctl(UNMAP_BUFFER, &(VA_BASE + 1).to_le_bytes(), budget(), |_| {
                Err(4)
            }),
            Err(4)
        );
        assert_eq!(a.usage(), (4096, 1));
        let mut pending = NvAddressSpace::default();
        assert_eq!(
            map(&mut pending, &view, &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0]),
            Err(3)
        );
        assert_eq!(a.ioctl(INITIALIZE, &[0; 40], budget(), |_| Err(4)), Err(8));
        for (at, value, error) in [(0, 2, 2), (4, 1, 4), (8, 12288, 2), (12, 1, 4), (16, 1, 4)] {
            let mut bad = [0; 40];
            bad[at..at + 4].copy_from_slice(&u32::to_le_bytes(value));
            assert_eq!(
                pending.ioctl(INITIALIZE, &bad, budget(), |_| Err(4)),
                Err(error)
            );
            assert!(pending.space.is_none());
        }
        // Explicit range must satisfy start <= split <= end <= 1 << 40.
        let ranged = |start: u64, end: u64, split: u64| {
            let mut args = bytes(&[1, 0, 65536, 0]);
            for v in [start, end, split] {
                args.extend_from_slice(&v.to_le_bytes());
            }
            args
        };
        assert_eq!(
            pending.ioctl(INITIALIZE, &ranged(1 << 26, 1 << 30, 1 << 34), budget(), |_| Err(4)),
            Err(4)
        );
        assert!(pending.space.is_none());
        let alternate = bytes(&[65536, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        pending
            .ioctl(INITIALIZE, &alternate, budget(), |_| Err(4))
            .unwrap();
    }
    #[test]
    fn explicit_va_range_bounds_mappings_and_reports_regions() {
        let mut a = NvAddressSpace::default();
        let mut args = bytes(&[1, 0, 65536, 0]);
        for v in [1u64 << 26, 1 << 37, 1 << 34] {
            args.extend_from_slice(&v.to_le_bytes());
        }
        a.ioctl(INITIALIZE, &args, budget(), |_| Err(4)).unwrap();
        let mut query = vec![0u8; 0x40];
        query[8..12].copy_from_slice(&48u32.to_le_bytes());
        let out = a.ioctl(GET_VA_REGIONS, &query, budget(), |_| Err(4)).unwrap();
        assert_eq!(word(&out, 8), 48);
        assert_eq!((wide(&out, 16), word(&out, 24), wide(&out, 32)), (1 << 26, 4096, ((1 << 34) - (1 << 26)) >> 12));
        assert_eq!((wide(&out, 40), word(&out, 48), wide(&out, 56)), (1 << 34, 65536, ((1 << 37) - (1 << 34)) >> 16));
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![0; 4096], Permissions::READ_WRITE).unwrap();
        let view = m.pin(0x8000, 4096, Permissions::READ_WRITE).unwrap();
        let out = map(&mut a, &view, &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(wide(&out, 32), 1 << 26);
    }
    #[test]
    fn mapping_limits_and_canceled_snapshots_leave_the_first_fit_address_available() {
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        let view = m.pin(0x8000, 4096, Permissions::READ_WRITE).unwrap();
        let args = bytes(&[0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        let mut a = init();
        for limit in [
            ImportBudget {
                bytes: 4095,
                spans: 1,
            },
            ImportBudget {
                bytes: 4096,
                spans: 0,
            },
        ] {
            assert_eq!(
                a.ioctl(MAP_BUFFER, &args, limit, |_| Ok(view.clone())),
                Err(6)
            );
            assert_eq!(a.usage(), (0, 0));
        }
        let mut canceled = a.clone();
        map(&mut canceled, &view, &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        drop(canceled);
        for n in 0..64 {
            let out = map(&mut a, &view, &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0]).unwrap();
            assert_eq!(wide(&out, 32), VA_BASE + n * 4096);
        }
        assert_eq!(map(&mut a, &view, &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0]), Err(6));
        a.ioctl(UNMAP_BUFFER, &VA_BASE.to_le_bytes(), budget(), |_| Err(4))
            .unwrap();
        let out = map(&mut a, &view, &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(wide(&out, 32), VA_BASE);
    }
}
