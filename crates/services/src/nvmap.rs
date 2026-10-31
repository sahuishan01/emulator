//! Original bounded unallocated nvmap objects. No backing import/allocation yet.
use std::collections::BTreeMap;
pub(crate) const NOT_SUPPORTED: u32 = 2;
pub(crate) const BAD_PARAMETER: u32 = 4;
pub(crate) const NO_MEMORY: u32 = 6;
#[derive(Clone, Debug)]
struct MapObject {
    size: u32,
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
    pub fn argument_size(code: u32) -> Option<usize> {
        match code {
            0xc0080101 => Some(8),
            0xc00c0109 => Some(12),
            0xc0180105 => Some(24),
            _ => None,
        }
    }
    pub fn ioctl(&mut self, code: u32, input: &[u8]) -> Result<Vec<u8>, u32> {
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
                if size == 0 || size % 4096 != 0 || size > 64 * 1024 * 1024 {
                    return Err(BAD_PARAMETER);
                }
                if self.objects.len() >= 64 {
                    return Err(NO_MEMORY);
                }
                let next = self.next.checked_add(1).ok_or(NO_MEMORY)?;
                let handle = self.next;
                self.objects.insert(handle, MapObject { size });
                self.next = next;
                output[4..8].copy_from_slice(&handle.to_le_bytes());
            }
            0xc00c0109 => {
                let object = self.objects.get(&word(0)).ok_or(BAD_PARAMETER)?;
                if word(4) != 1 {
                    return Err(NOT_SUPPORTED);
                }
                output[8..12].copy_from_slice(&object.size.to_le_bytes());
            }
            0xc0180105 => {
                if word(4) != 0 {
                    return Err(BAD_PARAMETER);
                }
                let object = self.objects.remove(&word(0)).ok_or(BAD_PARAMETER)?;
                output[8..16].fill(0);
                output[16..20].copy_from_slice(&object.size.to_le_bytes());
                output[20..24].fill(0);
            }
            _ => return Err(NOT_SUPPORTED),
        }
        Ok(output)
    }
}
