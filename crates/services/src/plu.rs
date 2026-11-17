//! Original `pl:u` (shared system font) service.
//!
//! The project ships its own system font (`sysdata/fonts`, DejaVu Sans) in
//! place of console firmware fonts, so no firmware dump is needed. One copy is
//! placed in the shared region and every font type points at it. Glyph
//! coverage is that font's: CJK and console symbol glyphs are absent.
//!
//! Region layout per font: an 8-byte header (`magic ^ key`, `size ^ key`,
//! both stored big-endian) followed by the TTF data with each 32-bit word
//! XORed with the key. Offsets reported to the guest point past the header.

use emulator_ipc::Request;
use emulator_kernel::service::{
    PreparedServiceEffect, Service, ServiceContext, ServiceError, ServiceReply,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Size of the Horizon shared font region.
pub const SHARED_FONT_REGION_SIZE: usize = 0x110_0000;
/// Standard, Chinese simplified, extended Chinese simplified, Chinese
/// traditional, Korean, Nintendo extended, Nintendo extended 2.
pub const SHARED_FONT_TYPES: u32 = 7;
/// `GetLoadState` value for a font that is not (yet) loaded.
pub const FONT_LOAD_STATE_LOADING: u32 = 0;
/// `GetLoadState` value for a loaded font.
pub const FONT_LOAD_STATE_LOADED: u32 = 1;
/// Plain-text value of the first header word.
pub const FONT_HEADER_MAGIC: u32 = 0x1802_9a7f;
/// Obfuscation key used for the bundled font. Any key decodes the same way.
pub const FONT_KEY: u32 = 0x49621806;
/// The bundled system font.
pub const SYSTEM_FONT: &[u8] = include_bytes!("../sysdata/fonts/DejaVuSans.ttf");
const RESULT_PL_INVALID_FONT_TYPE: u32 = 1 | (196 << 9);

/// Write one font at `offset` (header included) and return the data offset.
pub fn encode_font(region: &mut [u8], offset: usize, font: &[u8], key: u32) -> Option<usize> {
    let size = u32::try_from(font.len()).ok()?;
    let end = offset.checked_add(8)?.checked_add(font.len().next_multiple_of(4))?;
    if offset % 4 != 0 || end > region.len() {
        return None;
    }
    region[offset..offset + 4].copy_from_slice(&(FONT_HEADER_MAGIC ^ key).to_be_bytes());
    region[offset + 4..offset + 8].copy_from_slice(&(size ^ key).to_be_bytes());
    let key_bytes = key.to_be_bytes();
    for (i, slot) in region[offset + 8..end].iter_mut().enumerate() {
        *slot = font.get(i).copied().unwrap_or(0) ^ key_bytes[i % 4];
    }
    Some(offset + 8)
}

/// Inverse of [`encode_font`]: `data_offset` is the offset reported to guests.
pub fn decode_font(region: &[u8], data_offset: usize) -> Option<Vec<u8>> {
    let header = region.get(data_offset.checked_sub(8)?..data_offset)?;
    let word = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().expect("4"));
    let key = word(0) ^ FONT_HEADER_MAGIC;
    let size = (word(4) ^ key) as usize;
    let key_bytes = key.to_be_bytes();
    let body = region.get(data_offset..data_offset.checked_add(size)?)?;
    Some(body.iter().enumerate().map(|(i, b)| b ^ key_bytes[i % 4]).collect())
}

#[derive(Clone, Debug)]
pub struct SharedFontService {
    region: Rc<RefCell<Vec<u8>>>,
    data_offset: u32,
    size: u32,
}

impl Default for SharedFontService {
    fn default() -> Self {
        Self::with_font(SYSTEM_FONT).expect("bundled font fits the shared region")
    }
}

impl SharedFontService {
    /// Build the region around one font that serves every font type.
    pub fn with_font(font: &[u8]) -> Option<Self> {
        let mut region = vec![0; SHARED_FONT_REGION_SIZE];
        let data_offset = encode_font(&mut region, 0, font, FONT_KEY)?;
        Some(Self {
            region: Rc::new(RefCell::new(region)),
            data_offset: u32::try_from(data_offset).ok()?,
            size: u32::try_from(font.len()).ok()?,
        })
    }
}

fn font_type(request: &Request) -> Result<Option<u32>, ServiceError> {
    let raw = request.data.get(..4).ok_or(ServiceError::InvalidPayload {
        expected: 4,
        actual: request.data.len(),
    })?;
    let ty = u32::from_le_bytes(raw.try_into().expect("4 bytes"));
    Ok((ty < SHARED_FONT_TYPES).then_some(ty))
}

/// Writes for the three `GetSharedFontInOrderOfPriority` output buffers.
#[derive(Debug)]
struct ListWrites {
    writes: Vec<(emulator_memory::MemoryView, Vec<u8>)>,
    ranges: Vec<emulator_memory::BackingRange>,
}

impl PreparedServiceEffect for ListWrites {
    fn write_ranges(&self) -> &[emulator_memory::BackingRange] {
        &self.ranges
    }
    fn commit(self: Box<Self>) {
        for (view, bytes) in self.writes {
            view.write(0, &bytes).expect("prepared output pin");
        }
    }
}

impl Service for SharedFontService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        if request.command_id == 5 {
            // GetSharedFontInOrderOfPriority(language code): three B buffers
            // (types, offsets, sizes) -> loaded flag, count.
            context.require_buffers(0, 3)?;
            if request.data.len() < 8 {
                return Err(ServiceError::InvalidPayload {
                    expected: 8,
                    actual: request.data.len(),
                });
            }
            let mut writes = Vec::new();
            let mut count = SHARED_FONT_TYPES as usize;
            for index in 0..3 {
                count = count.min(context.receive_buffer(index)?.len() / 4);
            }
            for index in 0..3 {
                let view = context.receive_buffer(index)?;
                let bytes: Vec<u8> = (0..count as u32)
                    .flat_map(|ty| {
                        [ty, self.data_offset, self.size][index].to_le_bytes()
                    })
                    .collect();
                if !bytes.is_empty() {
                    let view = view.slice(0, bytes.len()).map_err(ServiceError::Memory)?;
                    writes.push((view, bytes));
                }
            }
            let ranges = writes.iter().flat_map(|(v, _)| v.backing_ranges()).collect();
            let mut data = vec![0u8; 8];
            data[0] = 1; // loaded
            data[4..8].copy_from_slice(&(count as u32).to_le_bytes());
            return Ok(ServiceReply {
                data,
                effect: Some(Box::new(ListWrites { writes, ranges })),
                ..ServiceReply::default()
            });
        }
        context.require_buffers(0, 0)?;
        self.dispatch(request)
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        let word = |value: u32| ServiceReply {
            data: value.to_le_bytes().to_vec(),
            ..ServiceReply::default()
        };
        let invalid = || ServiceReply {
            result: RESULT_PL_INVALID_FONT_TYPE,
            ..ServiceReply::default()
        };
        let per_type = |value: u32| font_type(request).map(|ty| ty.map_or_else(invalid, |_| word(value)));
        match request.command_id {
            // RequestLoad: fonts are resident from the start.
            0 => Ok(match font_type(request)? {
                Some(_) => ServiceReply::default(),
                None => invalid(),
            }),
            1 => per_type(FONT_LOAD_STATE_LOADED),
            2 => per_type(self.size),
            3 => per_type(self.data_offset),
            // GetSharedMemoryNativeHandle
            4 => Ok(ServiceReply {
                moved_objects: vec![Box::new(crate::hid::SharedMemoryHandle(Rc::clone(
                    &self.region,
                )))],
                ..ServiceReply::default()
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "pl:u",
                command,
            }),
        }
    }
}
