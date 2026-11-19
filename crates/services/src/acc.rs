//! Original `acc:u0` / `acc:u1` (account) service.
//!
//! The emulator has exactly one local user, [`DEFAULT_USER_ID`], with the
//! nickname [`DEFAULT_NICKNAME`]. It is the only registered user, the user
//! the application is opened with, and the user picked without interaction.
//! Network service accounts, profile images, thumbnails and user selection
//! UI are not implemented and fail explicitly.

use crate::BufferWrite;
use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};

/// The single local user's id (nonzero; zero is the invalid uid).
pub const DEFAULT_USER_ID: [u8; 16] = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// Nickname shown for [`DEFAULT_USER_ID`].
pub const DEFAULT_NICKNAME: &[u8] = b"Player";
/// `ProfileBase.last_edit_timestamp` (POSIX seconds) of the default user.
const PROFILE_EDIT_TIME: u64 = 1_767_225_600;
/// Size of `ProfileBase`: uid, last edit timestamp, 0x20-byte nickname.
pub const PROFILE_BASE_SIZE: usize = 0x38;
/// Size of `UserData` returned by IProfile::Get.
pub const USER_DATA_SIZE: usize = 0x80;
/// The uid argument names no registered user (acc module 124).
pub const RESULT_ACCOUNT_USER_NOT_FOUND: u32 = 124 | (100 << 9);

fn uid_argument(request: &Request) -> Result<[u8; 16], ServiceError> {
    let raw = request.data.get(..16).ok_or(ServiceError::InvalidPayload {
        expected: 16,
        actual: request.data.len(),
    })?;
    Ok(raw.try_into().expect("16 bytes"))
}

fn reply(data: Vec<u8>) -> ServiceReply {
    ServiceReply {
        data,
        ..ServiceReply::default()
    }
}

fn failure(result: u32) -> ServiceReply {
    ServiceReply {
        result,
        ..ServiceReply::default()
    }
}

/// Fills a C (pointer receive) buffer with `uids`, zero (invalid) after them.
fn uid_list(context: &ServiceContext<'_>, uids: &[[u8; 16]]) -> Result<ServiceReply, ServiceError> {
    let Some(view) = context.pointer_receive(0)? else {
        return Ok(ServiceReply::default());
    };
    let mut bytes = vec![0u8; view.len() - view.len() % 16];
    for (slot, uid) in bytes.chunks_exact_mut(16).zip(uids) {
        slot.copy_from_slice(uid);
    }
    if bytes.is_empty() {
        return Ok(ServiceReply::default());
    }
    let view = view.slice(0, bytes.len()).map_err(ServiceError::Memory)?;
    Ok(ServiceReply {
        effect: Some(Box::new(BufferWrite::new(view, bytes))),
        ..ServiceReply::default()
    })
}

#[derive(Clone, Debug, Default)]
pub struct AccountService;

impl Service for AccountService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        match request.command_id {
            // ListAllUsers / ListOpenUsers -> C: Uid[]
            2 | 3 => uid_list(context, &[DEFAULT_USER_ID]),
            // ListOpenContextStoredUsers -> C: Uid[] (none stored)
            60 => uid_list(context, &[]),
            _ => {
                context.require_buffers(0, 0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetUserCount -> i32
            0 => Ok(reply(1i32.to_le_bytes().to_vec())),
            // GetUserExistence(Uid) -> bool
            1 => Ok(reply(vec![u8::from(
                uid_argument(request)? == DEFAULT_USER_ID,
            )])),
            // GetLastOpenedUser -> Uid
            4 => Ok(reply(DEFAULT_USER_ID.to_vec())),
            // GetProfile(Uid) -> IProfile
            5 => {
                let user = uid_argument(request)?;
                if user != DEFAULT_USER_ID {
                    return Ok(failure(RESULT_ACCOUNT_USER_NOT_FOUND));
                }
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(ProfileService { user_id: user })],
                    ..ServiceReply::default()
                })
            }
            // GetBaasAccountManagerForApplication(Uid) -> IManagerForApplication
            101 => {
                let user = uid_argument(request)?;
                if user != DEFAULT_USER_ID {
                    return Ok(failure(RESULT_ACCOUNT_USER_NOT_FOUND));
                }
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(ManagerForApplication { user_id: user })],
                    ..ServiceReply::default()
                })
            }
            // IsUserRegistrationRequestPermitted(pid) -> bool: no user creation.
            50 => Ok(reply(vec![0])),
            // TrySelectUserWithoutInteraction(bool) -> Uid: the only user.
            51 => Ok(reply(DEFAULT_USER_ID.to_vec())),
            // InitializeApplicationInfoV0 / InitializeApplicationInfo /
            // InitializeApplicationInfoV2 (pid): binds the caller as the
            // application; nothing further is tracked per application.
            100 | 140 | 160 => Ok(ServiceReply::default()),
            // IsUserAccountSwitchLocked -> bool
            150 => Ok(reply(vec![0])),
            command => Err(ServiceError::UnsupportedCommand {
                service: "acc:u0",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProfileService {
    user_id: [u8; 16],
}

impl ProfileService {
    fn base(&self) -> Vec<u8> {
        let mut data = vec![0u8; PROFILE_BASE_SIZE];
        data[0..16].copy_from_slice(&self.user_id);
        data[0x10..0x18].copy_from_slice(&PROFILE_EDIT_TIME.to_le_bytes());
        data[0x18..0x18 + DEFAULT_NICKNAME.len()].copy_from_slice(DEFAULT_NICKNAME);
        data
    }
}

impl Service for ProfileService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        if request.command_id == 0 {
            // Get -> ProfileBase, C: UserData (default icon and colours).
            let mut reply = reply(self.base());
            if let Some(view) = context.pointer_receive(0)? {
                let length = view.len().min(USER_DATA_SIZE);
                if length > 0 {
                    let view = view.slice(0, length).map_err(ServiceError::Memory)?;
                    reply.effect = Some(Box::new(BufferWrite::new(view, vec![0; length])));
                }
            }
            return Ok(reply);
        }
        context.require_buffers(0, 0)?;
        self.dispatch(request)
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetBase -> ProfileBase
            1 => Ok(reply(self.base())),
            // GetImageSize / LoadImage need a profile image, which the
            // project does not ship yet.
            command => Err(ServiceError::UnsupportedCommand {
                service: "IProfile",
                command,
            }),
        }
    }
}

/// `IManagerForApplication` for a registered user. The user has no linked
/// network service account and the project has no account server, so every
/// network command (availability, account id, id tokens, authorization,
/// licenses) fails explicitly until it is modelled.
#[derive(Clone, Debug, Default)]
pub struct ManagerForApplication {
    user_id: [u8; 16],
}

impl ManagerForApplication {
    pub fn user_id(&self) -> [u8; 16] {
        self.user_id
    }
}

impl Service for ManagerForApplication {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        Err(ServiceError::UnsupportedCommand {
            service: "IManagerForApplication",
            command: request.command_id,
        })
    }
}
