//! Original bounded NV GPU address spaces backed by the shared GPU memory core.
use crate::nvmap::{ImportBudget, BAD_PARAMETER, IMPORT_BYTES, NOT_SUPPORTED, NO_MEMORY};
use emulator_gpu::{GpuAddressSpace, GpuMemoryError, GpuVa, MappingId};
use emulator_memory::MemoryView;

pub(crate) const BIND_CHANNEL: u32 = 0x40044101;
pub(crate) const INITIALIZE: u32 = 0x40284109;
pub(crate) const MAP_BUFFER: u32 = 0xc0284106;
pub(crate) const UNMAP_BUFFER: u32 = 0xc0084105;
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
}
impl Clone for NvAddressSpace {
    fn clone(&self) -> Self {
        Self {
            space: self
                .space
                .as_ref()
                .map(GpuAddressSpace::snapshot_for_transaction),
            regions: self.regions.clone(),
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
impl NvAddressSpace {
    pub fn argument_size(code: u32) -> Option<usize> {
        match code {
            BIND_CHANNEL => Some(4),
            INITIALIZE | MAP_BUFFER => Some(40),
            UNMAP_BUFFER => Some(8),
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
            if !(client_order || alternate_order) || input[16..].iter().any(|b| *b != 0) {
                return Err(NOT_SUPPORTED);
            }
            if word(input, 4) != 0 || word(input, 12) != 0 {
                return Err(BAD_PARAMETER);
            }
            self.space = Some(GpuAddressSpace::new(IMPORT_BYTES).map_err(memory_error)?);
            return Ok(output);
        }
        let space = self.space.as_mut().ok_or(3_u32)?;
        match code {
            MAP_BUFFER => {
                if ![0, 4].contains(&word(input, 0))
                    || ![0, u32::MAX].contains(&word(input, 4))
                    || ![0, 4096].contains(&word(input, 12))
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
                let align = wide(input, 32);
                if align != 0 && (!align.is_power_of_two() || align > IMPORT_BYTES as u64) {
                    return Err(BAD_PARAMETER);
                }
                let align = align.max(4096);
                if self.regions.len() >= 64 || length > budget.bytes {
                    return Err(NO_MEMORY);
                }
                let view = view.slice(offset, length).map_err(|_| BAD_PARAMETER)?;
                if view.backing_ranges().count() > budget.spans {
                    return Err(NO_MEMORY);
                }
                let aligned = |value: u64| {
                    value
                        .checked_add(align - 1)
                        .map(|v| v & !(align - 1))
                        .ok_or(NO_MEMORY)
                };
                let mut address = aligned(VA_BASE)?;
                for region in &self.regions {
                    if address.checked_add(length as u64).ok_or(NO_MEMORY)? <= region.address {
                        break;
                    }
                    if address < region.address + region.length as u64 {
                        address = aligned(region.address + region.length as u64)?;
                    }
                }
                if address.checked_add(length as u64).ok_or(NO_MEMORY)? > VA_END {
                    return Err(NO_MEMORY);
                }
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
                output[12..16].copy_from_slice(&4096_u32.to_le_bytes());
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
            ([1, 0, 1, 4096, 0, 0, 4096, 0, 0, 0], 2),
            ([0, 1, 1, 4096, 0, 0, 4096, 0, 0, 0], 2),
            ([0, 0, 1, 65536, 0, 0, 4096, 0, 0, 0], 2),
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
        for (at, value, error) in [(0, 2, 2), (4, 1, 4), (8, 12288, 2), (12, 1, 4), (16, 1, 2)] {
            let mut bad = [0; 40];
            bad[at..at + 4].copy_from_slice(&u32::to_le_bytes(value));
            assert_eq!(
                pending.ioctl(INITIALIZE, &bad, budget(), |_| Err(4)),
                Err(error)
            );
            assert!(pending.space.is_none());
        }
        let alternate = bytes(&[65536, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        pending
            .ioctl(INITIALIZE, &alternate, budget(), |_| Err(4))
            .unwrap();
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
