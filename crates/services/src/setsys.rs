//! Original `set:sys` / `set` (Settings System) service implementation.
//! Provides system language, device nickname, region codes, product model, and firmware version.

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
            1 => Ok(ServiceReply {
                result: 0,
                data: 1u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetSystemLanguage: returns SystemLanguage (u32)
            // 1 = AmericanEnglish
            2 => Ok(ServiceReply {
                result: 0,
                data: 1u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetColorSetId: 0 = BasicBlack (Dark Theme), 1 = BasicWhite
            3 => Ok(ServiceReply {
                result: 0,
                data: 0u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetPrimaryAlbumStorage: 1 = SdCard
            5 => Ok(ServiceReply {
                result: 0,
                data: 1u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetProductModel: 1 = Erista / Switch
            8 => Ok(ServiceReply {
                result: 0,
                data: 1u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetDeviceNickName: returns 0x80 bytes UTF-8 device name ("Switch")
            11 => {
                let mut name = vec![0u8; 0x80];
                name[..6].copy_from_slice(b"Switch");
                Ok(ServiceReply {
                    result: 0,
                    data: name,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetNotificationVolume: returns 1.0f32
            13 => {
                let vol = 1.0f32;
                Ok(ServiceReply {
                    result: 0,
                    data: vol.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetFirmwareVersion: returns 0x100 byte firmware version descriptor
            14 => {
                let mut fw = vec![0u8; 0x100];
                fw[0] = 18; // major
                fw[1] = 1; // minor
                fw[2] = 0; // micro
                fw[3] = 0;
                fw[4] = 0; // revision_major
                fw[5] = 0; // revision_minor
                fw[0x08..0x08 + 2].copy_from_slice(b"NX");
                fw[0x28..0x28 + 6].copy_from_slice(b"18.1.0");
                fw[0x68..0x68 + 15].copy_from_slice(b"Nintendo Switch");
                Ok(ServiceReply {
                    result: 0,
                    data: fw,
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
