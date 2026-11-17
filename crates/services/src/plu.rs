//! Original `pl:u` (shared system font) service.
//!
//! Real consoles load the system fonts from firmware into a shared-memory
//! region. No firmware fonts are bundled or supplied yet, so this service
//! exposes the region and the font table but **never reports a font as
//! loaded**: load state stays "loading", sizes and offsets are 0 and the
//! priority list is empty. Callers see honest "not available" answers
//! instead of fabricated font data.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use std::cell::RefCell;
use std::rc::Rc;

/// Size of the Horizon shared font region.
pub const SHARED_FONT_REGION_SIZE: usize = 0x110_0000;
/// Standard, Chinese simplified, extended Chinese simplified, Chinese
/// traditional, Korean, Nintendo extended, Nintendo extended 2.
pub const SHARED_FONT_TYPES: u32 = 7;
/// `GetLoadState` value for a font that is not (yet) loaded.
pub const FONT_LOAD_STATE_LOADING: u32 = 0;
const RESULT_PL_INVALID_FONT_TYPE: u32 = 1 | (196 << 9);

#[derive(Clone, Debug)]
pub struct SharedFontService {
    region: Rc<RefCell<Vec<u8>>>,
}

impl Default for SharedFontService {
    fn default() -> Self {
        Self {
            region: Rc::new(RefCell::new(vec![0; SHARED_FONT_REGION_SIZE])),
        }
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
            // (types, offsets, sizes) -> loaded flag, count. No fonts.
            context.require_buffers(0, 3)?;
            if request.data.len() < 8 {
                return Err(ServiceError::InvalidPayload {
                    expected: 8,
                    actual: request.data.len(),
                });
            }
            let mut data = vec![0u8; 8];
            data[0] = 0; // not loaded
            data[4..8].copy_from_slice(&0u32.to_le_bytes());
            return Ok(ServiceReply {
                data,
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
        let invalid = ServiceReply {
            result: RESULT_PL_INVALID_FONT_TYPE,
            ..ServiceReply::default()
        };
        match request.command_id {
            // RequestLoad: accepted, but nothing can be loaded.
            0 => Ok(match font_type(request)? {
                Some(_) => ServiceReply::default(),
                None => invalid,
            }),
            // GetLoadState / GetSize / GetSharedMemoryAddressOffset.
            1 => Ok(font_type(request)?.map_or(invalid, |_| word(FONT_LOAD_STATE_LOADING))),
            2 | 3 => Ok(font_type(request)?.map_or(invalid, |_| word(0))),
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
