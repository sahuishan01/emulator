//! Original bounded NVMap objects and imports of existing guest backing.
use emulator_memory::{MemoryView, Permissions};
use std::collections::BTreeMap;

pub(crate) const NOT_SUPPORTED: u32 = 2;
pub(crate) const BAD_PARAMETER: u32 = 4;
pub(crate) const NO_MEMORY: u32 = 6;
const INVALID_STATE: u32 = 8;
pub(crate) const IMPORT_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const IMPORT_SPANS: usize = 4096;

#[derive(Clone, Copy)]
pub(crate) struct ImportBudget {
    pub bytes: usize,
    pub spans: usize,
}
#[derive(Clone, Debug)]
struct Allocation {
    view: MemoryView,
    address: u64,
    alignment: u32,
}
#[derive(Clone, Debug)]
struct MapObject {
    size: u32,
    owner: u32,
    allocation: Option<Allocation>,
}
#[derive(Clone, Debug)]
pub(crate) struct NvMap {
    objects: BTreeMap<u32, MapObject>,
    next: u32,
}
impl Default for NvMap {
    fn default() -> Self {
        Self {
            objects: BTreeMap::new(),
            next: 1,
        }
    }
}
impl NvMap {
    pub fn retain_owners(&mut self, owners: &[u32]) {
        self.objects
            .retain(|_, object| owners.contains(&object.owner));
    }
    pub fn imported_view(&self, handle: u32) -> Result<MemoryView, u32> {
        self.objects
            .get(&handle)
            .ok_or(BAD_PARAMETER)?
            .allocation
            .as_ref()
            .map(|a| a.view.clone())
            .ok_or(INVALID_STATE)
    }
    pub fn imported_usage(&self) -> (usize, usize) {
        self.objects
            .values()
            .filter_map(|o| o.allocation.as_ref())
            .fold((0, 0), |(bytes, spans), a| {
                (
                    bytes + a.view.len(),
                    spans + a.view.backing_ranges().count(),
                )
            })
    }
    pub fn argument_size(code: u32) -> Option<usize> {
        match code {
            0xc0080101 => Some(8),
            0xc0200104 => Some(32),
            0xc00c0109 => Some(12),
            0xc0180105 => Some(24),
            _ => None,
        }
    }
    // The capability supplier runs only during preparation. No callback or
    // memory access is needed when the isolated metadata snapshot is published.
    pub fn ioctl(
        &mut self,
        fd: u32,
        code: u32,
        input: &[u8],
        budget: ImportBudget,
        pin: impl FnOnce(u64, usize, Permissions) -> Option<MemoryView>,
    ) -> Result<Vec<u8>, u32> {
        if Some(input.len()) != Self::argument_size(code) {
            return Err(BAD_PARAMETER);
        }
        let word = |offset: usize| {
            u32::from_le_bytes(
                input[offset..offset + 4]
                    .try_into()
                    .expect("validated ioctl word"),
            )
        };
        let mut output = input.to_vec();
        match code {
            0xc0080101 => {
                let size = word(0);
                if size == 0 || size % 4096 != 0 || size as usize > IMPORT_BYTES {
                    return Err(BAD_PARAMETER);
                }
                if self.objects.values().filter(|o| o.owner == fd).count() >= 64 {
                    return Err(NO_MEMORY);
                }
                let next = self.next.checked_add(1).ok_or(NO_MEMORY)?;
                let handle = self.next;
                self.objects.insert(
                    handle,
                    MapObject {
                        size,
                        owner: fd,
                        allocation: None,
                    },
                );
                self.next = next;
                output[4..8].copy_from_slice(&handle.to_le_bytes());
            }
            0xc0200104 => {
                let object = self
                    .objects
                    .get_mut(&word(0))
                    .filter(|o| o.owner == fd)
                    .ok_or(BAD_PARAMETER)?;
                if object.allocation.is_some() {
                    return Err(INVALID_STATE);
                }
                let address =
                    u64::from_le_bytes(input[24..32].try_into().expect("validated address"));
                if word(4) != 0 || word(8) > 1 || input[16] != 0 || address == 0 {
                    return Err(NOT_SUPPORTED);
                }
                let requested = word(12);
                if (requested != 0 && !requested.is_power_of_two()) || input[17..24] != [0; 7] {
                    return Err(BAD_PARAMETER);
                }
                let alignment = requested.max(4096);
                if alignment as usize > IMPORT_BYTES || address % u64::from(alignment) != 0 {
                    return Err(BAD_PARAMETER);
                }
                if object.size as usize > budget.bytes {
                    return Err(NO_MEMORY);
                }
                let permissions = if word(8) == 0 {
                    Permissions::READ
                } else {
                    Permissions::READ_WRITE
                };
                let view = pin(address, object.size as usize, permissions).ok_or(BAD_PARAMETER)?;
                // Validate supplied capabilities as well as caller arguments.
                if view.len() != object.size as usize || view.permissions() != permissions {
                    return Err(BAD_PARAMETER);
                }
                if view.backing_ranges().count() > budget.spans {
                    return Err(NO_MEMORY);
                }
                object.allocation = Some(Allocation {
                    view,
                    address,
                    alignment,
                });
                output[12..16].copy_from_slice(&alignment.to_le_bytes());
            }
            0xc00c0109 => {
                let object = self
                    .objects
                    .get(&word(0))
                    .filter(|o| o.owner == fd)
                    .ok_or(BAD_PARAMETER)?;
                let value = match word(4) {
                    1 => object.size,
                    2 => object.allocation.as_ref().ok_or(INVALID_STATE)?.alignment,
                    5 => {
                        object.allocation.as_ref().ok_or(INVALID_STATE)?;
                        0
                    }
                    _ => return Err(NOT_SUPPORTED),
                };
                output[8..12].copy_from_slice(&value.to_le_bytes());
            }
            0xc0180105 => {
                if word(4) != 0 {
                    return Err(BAD_PARAMETER);
                }
                self.objects
                    .get(&word(0))
                    .filter(|o| o.owner == fd)
                    .ok_or(BAD_PARAMETER)?;
                let object = self.objects.remove(&word(0)).expect("checked owned object");
                let address = object.allocation.as_ref().map_or(0, |a| a.address);
                output[8..16].copy_from_slice(&address.to_le_bytes());
                output[16..20].copy_from_slice(&object.size.to_le_bytes());
                output[20..24].fill(0);
            }
            _ => return Err(NOT_SUPPORTED),
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emulator_memory::AddressSpace;
    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
    fn budget() -> ImportBudget {
        ImportBudget {
            bytes: IMPORT_BYTES,
            spans: IMPORT_SPANS,
        }
    }
    fn call(n: &mut NvMap, m: &AddressSpace, code: u32, args: &[u32]) -> Result<Vec<u8>, u32> {
        n.ioctl(1, code, &words(args), budget(), |a, s, p| {
            m.pin(a, s, p).ok()
        })
    }
    fn object() -> NvMap {
        let mut n = NvMap::default();
        n.ioctl(1, 0xc0080101, &words(&[4096, 0]), budget(), |_, _, _| None)
            .unwrap();
        n
    }
    #[test]
    fn imported_backing_observes_cpu_writes_and_survives_unmap_and_address_reuse() {
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![0; 8192], Permissions::READ_WRITE)
            .unwrap();
        let mut n = object();
        let out = call(&mut n, &m, 0xc0200104, &[1, 0, 1, 0, 0, 0, 0x8000, 0]).unwrap();
        assert_eq!(u32::from_le_bytes(out[12..16].try_into().unwrap()), 4096);
        assert_eq!(n.imported_usage(), (4096, 1));
        m.write_u64(0x8000, 42).unwrap();
        m.unmap(0x8000, 8192).unwrap();
        assert_eq!(m.resident_bytes(), 8192);
        m.map(0x8000, vec![99; 8192], Permissions::READ_WRITE)
            .unwrap();
        let mut value = [0; 8];
        n.objects[&1]
            .allocation
            .as_ref()
            .unwrap()
            .view
            .read(0, &mut value)
            .unwrap();
        assert_eq!(u64::from_le_bytes(value), 42);
        n.objects[&1]
            .allocation
            .as_ref()
            .unwrap()
            .view
            .write(0, &77_u64.to_le_bytes())
            .unwrap();
        assert_eq!(m.read_u64(0x8000).unwrap(), 0x6363636363636363);
        assert_eq!(m.resident_bytes(), 16384);
        let free = call(&mut n, &m, 0xc0180105, &[1, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(u64::from_le_bytes(free[8..16].try_into().unwrap()), 0x8000);
        assert_eq!(n.imported_usage(), (0, 0));
        assert_eq!(m.resident_bytes(), 8192);
        m.unmap(0x8000, 8192).unwrap();
        assert_eq!(m.resident_bytes(), 0);
    }
    #[test]
    fn readonly_import_cannot_gain_write_rights_and_failed_import_leaves_object_unallocated() {
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![42; 4096], Permissions::READ).unwrap();
        let mut n = object();
        assert_eq!(call(&mut n, &m, 0xc00c0109, &[1, 2, 0]), Err(INVALID_STATE));
        assert_eq!(
            call(&mut n, &m, 0xc0200104, &[1, 0, 1, 4096, 0, 0, 0x8000, 0]),
            Err(BAD_PARAMETER)
        );
        assert_eq!(n.imported_usage(), (0, 0));
        call(&mut n, &m, 0xc0200104, &[1, 0, 0, 4096, 0, 0, 0x8000, 0]).unwrap();
        let kind = call(&mut n, &m, 0xc00c0109, &[1, 5, 99]).unwrap();
        assert_eq!(&kind[8..12], &[0; 4]);
        assert!(n.objects[&1]
            .allocation
            .as_ref()
            .unwrap()
            .view
            .write(0, &[99])
            .is_err());
        assert_eq!(
            call(&mut n, &m, 0xc0200104, &[1, 0, 0, 4096, 0, 0, 0x8000, 0]),
            Err(INVALID_STATE)
        );
    }
    #[test]
    fn import_rejects_unsupported_modes_bad_alignment_padding_addresses_and_capabilities() {
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![0; 8192], Permissions::READ_WRITE)
            .unwrap();
        let mut n = object();
        for (args, error) in [
            ([1, 1, 1, 4096, 0, 0, 0x8000, 0], NOT_SUPPORTED),
            ([1, 0, 2, 4096, 0, 0, 0x8000, 0], NOT_SUPPORTED),
            ([1, 0, 1, 4096, 1, 0, 0x8000, 0], NOT_SUPPORTED),
            ([1, 0, 1, 3, 0, 0, 0x8000, 0], BAD_PARAMETER),
            ([1, 0, 1, 4096, 0x100, 0, 0x8000, 0], BAD_PARAMETER),
            ([1, 0, 1, 4096, 0, 0, 0x8001, 0], BAD_PARAMETER),
            ([1, 0, 1, 4096, 0, 0, 0, 0], NOT_SUPPORTED),
            ([1, 0, 1, 4096, 0, 0, 0x10000, 0], BAD_PARAMETER),
            ([1, 0, 1, 4096, 0, 0, 0xfffff000, 0xffffffff], BAD_PARAMETER),
        ] {
            assert_eq!(call(&mut n, &m, 0xc0200104, &args), Err(error));
            assert_eq!(n.imported_usage(), (0, 0));
        }
        let args = words(&[1, 0, 0, 4096, 0, 0, 0x8000, 0]);
        for (size, rights) in [(4096, Permissions::READ_WRITE), (8192, Permissions::READ)] {
            assert_eq!(
                n.ioctl(1, 0xc0200104, &args, budget(), |a, _, _| m
                    .pin(a, size, rights)
                    .ok()),
                Err(BAD_PARAMETER)
            );
        }
        assert_eq!(n.imported_usage(), (0, 0));
    }
    #[test]
    fn import_bytes_and_spans_limits_cancel_without_retaining_views() {
        let mut m = AddressSpace::default();
        m.map(0x8000, vec![0; 4096], Permissions::READ_WRITE)
            .unwrap();
        let mut n = object();
        let args = words(&[1, 0, 1, 4096, 0, 0, 0x8000, 0]);
        assert_eq!(
            n.ioctl(
                1,
                0xc0200104,
                &args,
                ImportBudget {
                    bytes: 4095,
                    spans: 1
                },
                |_, _, _| panic!("byte cap must precede pinning")
            ),
            Err(NO_MEMORY)
        );
        assert_eq!(
            n.ioctl(
                1,
                0xc0200104,
                &args,
                ImportBudget {
                    bytes: 4096,
                    spans: 0
                },
                |a, s, p| m.pin(a, s, p).ok()
            ),
            Err(NO_MEMORY)
        );
        assert_eq!(n.imported_usage(), (0, 0));
        let lease = m.lend(0x8000, 4096, Permissions::NONE).unwrap();
        drop(lease);
        call(&mut n, &m, 0xc0200104, &[1, 0, 1, 4096, 0, 0, 0x8000, 0]).unwrap();
    }
}
