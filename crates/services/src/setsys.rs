//! Original `set:sys` (Settings System) service implementation.
//! Provides system language, device nickname, and region codes.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct SetSysService;

impl Service for SetSysService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetLanguageCode: returns LanguageCode (u64)
            // Default "en-US" = 0x000053552d6e65
            0 => {
                let lang_code = b"en-US\0\0\0";
                Ok(ServiceReply {
                    result: 0,
                    data: lang_code.to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetRegionCode: returns RegionCode (u32)
            // 1 = USA, 2 = Europe, 0 = Japan
            1 => {
                Ok(ServiceReply {
                    result: 0,
                    data: 1u32.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "set:sys",
                command,
            }),
        }
    }
}
