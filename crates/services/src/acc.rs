//! Original `acc:u0` (Account) service implementation.
//! Provides user profile queries and authentication tokens.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct AccountService;

#[derive(Clone, Debug, Default)]
pub struct ProfileService {
    user_id: [u8; 16],
}

impl Service for ProfileService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Get (command 0): returns ProfileBase (UserData)
            0 => {
                let mut data = vec![0u8; 0x38];
                data[0..16].copy_from_slice(&self.user_id);
                data[0x10..0x18].copy_from_slice(&0x1000u64.to_le_bytes());
                let username = b"Player\0";
                data[0x18..0x18 + username.len()].copy_from_slice(username);
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetBase (command 1): returns UserData
            1 => {
                let mut data = vec![0u8; 0x38];
                data[0..16].copy_from_slice(&self.user_id);
                data[0x10..0x18].copy_from_slice(&0x1000u64.to_le_bytes());
                let username = b"Player\0";
                data[0x18..0x18 + username.len()].copy_from_slice(username);
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetImageSize (command 10)
            10 => {
                let size = 0x10000u32;
                Ok(ServiceReply {
                    result: 0,
                    data: size.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // LoadImage (command 11)
            11 => Ok(ServiceReply::default()),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IProfile",
                command,
            }),
        }
    }
}

impl Service for AccountService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetUserCount (command 0) -> returns count: i32
            0 => {
                let count = 1i32;
                Ok(ServiceReply {
                    result: 0,
                    data: count.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetUserExistence (command 1) -> returns exists: bool (1)
            1 => {
                let exists = 1u32;
                Ok(ServiceReply {
                    result: 0,
                    data: exists.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // ListAllUsers (command 2) -> returns list of 16-byte user UUIDs
            2 => {
                let mut user_id = [0u8; 16];
                user_id[0] = 1;
                Ok(ServiceReply {
                    result: 0,
                    data: user_id.to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // OpenUser (command 3) -> returns IProfile object
            3 => {
                let mut user_id = [0u8; 16];
                if request.data.len() >= 16 {
                    user_id.copy_from_slice(&request.data[0..16]);
                } else {
                    user_id[0] = 1;
                }
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(ProfileService { user_id })],
                    effect: None,
                })
            }
            // GetLastOpenedUser (command 4) -> returns 16-byte UUID
            4 => {
                let mut user_id = [0u8; 16];
                user_id[0] = 1;
                Ok(ServiceReply {
                    result: 0,
                    data: user_id.to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // InitializeApplicationInfo (command 5)
            5 => Ok(ServiceReply::default()),
            // GetSelectedUser (command 6)
            6 => {
                let mut user_id = [0u8; 16];
                user_id[0] = 1;
                Ok(ServiceReply {
                    result: 0,
                    data: user_id.to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // IsUserRegistrationRequestPermitted (command 50)
            50 => {
                let permitted = 0u32;
                Ok(ServiceReply {
                    result: 0,
                    data: permitted.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // InitializeApplicationInfoV2 (command 100)
            100 => Ok(ServiceReply::default()),
            command => Err(ServiceError::UnsupportedCommand {
                service: "acc:u0",
                command,
            }),
        }
    }
}
