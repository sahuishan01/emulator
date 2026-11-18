//! Original `appletOE` / `appletAE` (Applet Manager) service implementation.
//! Manages game execution lifecycle, focus handling, operation mode, and system controllers.
//!
//! One [`AppletState`] per process holds the applet message queue and the
//! launch parameter channels. The application starts in focus with one
//! FocusStateChanged message queued, and with a preselected-user launch
//! parameter naming [`DEFAULT_USER_ID`], the only local user (Horizon pushes
//! it for titles that require a user at startup; the NACP is not parsed, so
//! every title gets it). The user channel starts empty. Library applets,
//! transfer storages and the debug interface fail explicitly.

use crate::acc::DEFAULT_USER_ID;
use crate::event::EventHandle;
use emulator_ipc::Request;
use emulator_kernel::service::{
    PreparedServiceEffect, Service, ServiceContext, ServiceError, ServiceReply,
};
use emulator_memory::BackingRange;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

/// AM (module 128) desc 2: the launch parameter channel is empty.
pub const RESULT_AM_NO_DATA_IN_CHANNEL: u32 = 128 | (2 << 9);
/// AM desc 3: ReceiveMessage found no queued message.
pub const RESULT_AM_NO_MESSAGES: u32 = 128 | (3 << 9);
/// AM desc 503: storage access outside the storage.
pub const RESULT_AM_INVALID_OFFSET: u32 = 128 | (503 << 9);

/// Applet message: the focus state changed (read it with GetCurrentFocusState).
pub const MESSAGE_FOCUS_STATE_CHANGED: u32 = 15;
/// PopLaunchParameter kinds.
pub const LAUNCH_PARAMETER_USER_CHANNEL: u32 = 1;
pub const LAUNCH_PARAMETER_PRESELECTED_USER: u32 = 2;
/// `PreselectedUser` storage: magic, is_account_selected, uid, 0x70 unused.
pub const PRESELECTED_USER_MAGIC: u32 = 0xc794_97ca;
pub const PRESELECTED_USER_SIZE: usize = 0x88;

/// The preselected-user launch parameter for `user`.
pub fn preselected_user_storage(user: [u8; 16]) -> Vec<u8> {
    let mut data = vec![0u8; PRESELECTED_USER_SIZE];
    data[0..4].copy_from_slice(&PRESELECTED_USER_MAGIC.to_le_bytes());
    data[4] = 1;
    data[8..24].copy_from_slice(&user);
    data
}

/// Process-wide applet state shared by every appletOE/appletAE session.
#[derive(Debug)]
pub struct AppletState {
    messages: VecDeque<u32>,
    /// Signaled exactly while `messages` is non-empty.
    message_event: Rc<Cell<bool>>,
    preselected_user: VecDeque<Vec<u8>>,
    user_channel: VecDeque<Vec<u8>>,
}

impl Default for AppletState {
    fn default() -> Self {
        Self {
            messages: VecDeque::from([MESSAGE_FOCUS_STATE_CHANGED]),
            message_event: Rc::new(Cell::new(true)),
            preselected_user: VecDeque::from([preselected_user_storage(DEFAULT_USER_ID)]),
            user_channel: VecDeque::new(),
        }
    }
}

pub type SharedAppletState = Rc<RefCell<AppletState>>;

/// Applet state changes are applied only once the reply is committed.
#[derive(Debug)]
enum AppletCommit {
    PopMessage(SharedAppletState),
    PopLaunchParameter(SharedAppletState, u32),
    StorageWrite(Rc<RefCell<Vec<u8>>>, usize, Vec<u8>),
}

impl PreparedServiceEffect for AppletCommit {
    fn write_ranges(&self) -> &[BackingRange] {
        &[]
    }
    fn commit(self: Box<Self>) {
        match *self {
            AppletCommit::PopMessage(state) => {
                let mut state = state.borrow_mut();
                state.messages.pop_front();
                state.message_event.set(!state.messages.is_empty());
            }
            AppletCommit::PopLaunchParameter(state, kind) => {
                let mut state = state.borrow_mut();
                match kind {
                    LAUNCH_PARAMETER_PRESELECTED_USER => state.preselected_user.pop_front(),
                    _ => state.user_channel.pop_front(),
                };
            }
            AppletCommit::StorageWrite(data, offset, bytes) => {
                data.borrow_mut()[offset..offset + bytes.len()].copy_from_slice(&bytes);
            }
        }
    }
}

fn reply(data: Vec<u8>) -> ServiceReply {
    ServiceReply { data, ..ServiceReply::default() }
}

fn failure(result: u32) -> ServiceReply {
    ServiceReply { result, ..ServiceReply::default() }
}

fn object(service: impl Service + 'static) -> ServiceReply {
    ServiceReply { moved_objects: vec![Box::new(service)], ..ServiceReply::default() }
}

fn u32_argument(request: &Request) -> Result<u32, ServiceError> {
    crate::payload(request, 4)?;
    Ok(u32::from_le_bytes(request.data[..4].try_into().expect("4 bytes")))
}

#[derive(Clone, Debug, Default)]
pub struct AppletManagerService {
    state: SharedAppletState,
}

impl AppletManagerService {
    pub fn with_state(state: SharedAppletState) -> Self {
        Self { state }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommonStateGetter {
    state: SharedAppletState,
}

impl Service for CommonStateGetter {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetEventHandle: signaled while a message is queued.
            0 => {
                crate::payload(request, 0)?;
                let event = self.state.borrow().message_event.clone();
                Ok(object(EventHandle(event)))
            }
            // ReceiveMessage -> u32, or NoMessages once the queue is empty.
            1 => {
                crate::payload(request, 0)?;
                let Some(&message) = self.state.borrow().messages.front() else {
                    return Ok(failure(RESULT_AM_NO_MESSAGES));
                };
                Ok(ServiceReply {
                    data: message.to_le_bytes().to_vec(),
                    effect: Some(Box::new(AppletCommit::PopMessage(self.state.clone()))),
                    ..ServiceReply::default()
                })
            }
            // GetOperationMode: 0 = Handheld, 1 = Docked
            5 | 21 => {
                let mode = 0u8; // Handheld default
                Ok(ServiceReply {
                    result: 0,
                    data: vec![mode],
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetPerformanceMode: 0 = Handheld, 1 = Docked
            6 | 22 => {
                let perf_mode = 0u32;
                Ok(ServiceReply {
                    result: 0,
                    data: perf_mode.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetCurrentFocusState: 1 = InFocus, 2 = OutOfFocus
            9 => {
                let focus = 1u8; // InFocus
                Ok(ServiceReply {
                    result: 0,
                    data: vec![focus],
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetRequestExitToLibraryAppletAtExecuteNextProgramEnabled: a flag
            // with no effect here because no library applet ever requests exit.
            900 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply::default())
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "ICommonStateGetter",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SelfController;

impl Service for SelfController {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Exit, LockExit, UnlockExit
            0 | 1 | 2 => Ok(ServiceReply::default()),
            // GetLibraryAppletLaunchableEvent
            9 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(EventHandle::new(true))],
                effect: None,
            }),
            // SetScreenShotPermission, SetOperationModeChangedNotification, SetPerformanceModeChangedNotification
            10 | 11 | 12 => Ok(ServiceReply::default()),
            // SetFocusHandlingMode, SetRestartMessageEnabled, SetOutOfFocusSuspendingEnabled
            13 | 14 | 16 => Ok(ServiceReply::default()),
            // CreateManagedDisplayLayer: returns LayerId (u64)
            40 => {
                let layer_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: layer_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // CreateManagedDisplaySeparableLayer (10.0.0+): the display layer
            // plus a recording (capture) layer. The SDK requires both to be
            // valid, openable layers.
            44 => {
                let mut data = 1u64.to_le_bytes().to_vec();
                data.extend_from_slice(&2u64.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetHandlesRequestToDisplay, SetAlbumImageOrientation
            50 | 62 => Ok(ServiceReply::default()),
            // GetAccumulatedSuspendedTickValue: the guest is never suspended.
            90 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply {
                    data: 0_u64.to_le_bytes().to_vec(),
                    ..ServiceReply::default()
                })
            }
            // GetAccumulatedSuspendedTickChangedEvent: never signaled, because
            // the accumulated value above never changes.
            91 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(EventHandle::new(false))],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "ISelfController",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowController;

impl Service for WindowController {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetAppletResourceUserId: returns u64
            1 => {
                let user_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: user_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // AcquireForegroundRights
            10 => Ok(ServiceReply::default()),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IWindowController",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AudioController;

impl Service for AudioController {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // SetExpectedMasterVolume
            0 => Ok(ServiceReply::default()),
            // GetMainAppletExpectedMasterVolume: returns 1.0f32
            1 => {
                let vol = 1.0f32;
                Ok(ServiceReply {
                    result: 0,
                    data: vol.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IAudioController",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DisplayController;

impl Service for DisplayController {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        // Screen capture and the caller/last-frame buffers are not implemented.
        Err(ServiceError::UnsupportedCommand {
            service: "IDisplayController",
            command: request.command_id,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct LibraryAppletCreator;

impl Service for LibraryAppletCreator {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        // No library applets (keyboard, error, user selection, ...) exist yet,
        // so neither applets nor their argument storages can be created.
        Err(ServiceError::UnsupportedCommand {
            service: "ILibraryAppletCreator",
            command: request.command_id,
        })
    }
}

/// `IStorage`: a byte buffer passed through an applet channel.
#[derive(Clone, Debug)]
pub struct AppletStorage {
    data: Rc<RefCell<Vec<u8>>>,
}

impl AppletStorage {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data: Rc::new(RefCell::new(data)) }
    }
}

impl Service for AppletStorage {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // Open -> IStorageAccessor over the same bytes.
            0 => {
                crate::payload(request, 0)?;
                Ok(object(AppletStorageAccessor { data: self.data.clone() }))
            }
            // OpenTransferStorage is only valid for transfer-memory storages,
            // which are never created here.
            command => Err(ServiceError::UnsupportedCommand { service: "IStorage", command }),
        }
    }
}

/// `IStorageAccessor`: sized reads and writes into an [`AppletStorage`].
#[derive(Clone, Debug)]
pub struct AppletStorageAccessor {
    data: Rc<RefCell<Vec<u8>>>,
}

impl AppletStorageAccessor {
    /// `offset` and the buffer must lie entirely inside the storage.
    fn range(&self, request: &Request, length: usize) -> Result<Option<usize>, ServiceError> {
        crate::payload(request, 8)?;
        let offset = i64::from_le_bytes(request.data[..8].try_into().expect("8 bytes"));
        let size = self.data.borrow().len();
        Ok(usize::try_from(offset)
            .ok()
            .filter(|&offset| offset <= size && length <= size - offset))
    }
}

impl Service for AppletStorageAccessor {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(&mut self, request: &Request, context: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        match request.command_id {
            // Write(i64 offset, AutoSelect in buffer)
            10 => {
                context.require_auto_buffers(1, 0)?;
                let view = context.auto_send(0)?;
                let length = view.as_ref().map_or(0, |view| view.len());
                let Some(offset) = self.range(request, length)? else {
                    return Ok(failure(RESULT_AM_INVALID_OFFSET));
                };
                let mut bytes = vec![0u8; length];
                if let Some(view) = view {
                    view.read(0, &mut bytes).map_err(ServiceError::Memory)?;
                }
                Ok(ServiceReply {
                    effect: Some(Box::new(AppletCommit::StorageWrite(self.data.clone(), offset, bytes))),
                    ..ServiceReply::default()
                })
            }
            // Read(i64 offset, AutoSelect out buffer): fills the whole buffer.
            11 => {
                context.require_auto_buffers(0, 1)?;
                let view = context.auto_receive(0)?;
                let length = view.as_ref().map_or(0, |view| view.len());
                let Some(offset) = self.range(request, length)? else {
                    return Ok(failure(RESULT_AM_INVALID_OFFSET));
                };
                let Some(view) = view.filter(|_| length > 0) else {
                    return Ok(ServiceReply::default());
                };
                let bytes = self.data.borrow()[offset..offset + length].to_vec();
                Ok(ServiceReply {
                    effect: Some(Box::new(crate::BufferWrite::new(view, bytes))),
                    ..ServiceReply::default()
                })
            }
            _ => {
                context.require_buffers(0, 0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetSize -> i64
            0 => {
                crate::payload(request, 0)?;
                let size = self.data.borrow().len() as i64;
                Ok(reply(size.to_le_bytes().to_vec()))
            }
            command => Err(ServiceError::UnsupportedCommand { service: "IStorageAccessor", command }),
        }
    }
}

/// `IApplicationFunctions`, following the real command table. Commands not
/// listed (application launch, save data resizing, recording, JIT, ...) fail
/// explicitly.
#[derive(Clone, Debug, Default)]
pub struct ApplicationFunctions {
    state: SharedAppletState,
}

impl Service for ApplicationFunctions {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // PopLaunchParameter(u32 kind) -> IStorage
            1 => {
                let kind = u32_argument(request)?;
                let state = self.state.borrow();
                let channel = match kind {
                    LAUNCH_PARAMETER_USER_CHANNEL => &state.user_channel,
                    LAUNCH_PARAMETER_PRESELECTED_USER => &state.preselected_user,
                    _ => {
                        return Err(ServiceError::DeviceFault(format!(
                            "PopLaunchParameter: unknown launch parameter kind {kind}"
                        )));
                    }
                };
                let Some(data) = channel.front() else {
                    return Ok(failure(RESULT_AM_NO_DATA_IN_CHANNEL));
                };
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(AppletStorage::new(data.clone()))],
                    effect: Some(Box::new(AppletCommit::PopLaunchParameter(self.state.clone(), kind))),
                    ..ServiceReply::default()
                })
            }
            // EnsureSaveData(Uid) -> u64 required extra size: save data is
            // created on demand by fsp-srv, so nothing more is ever needed.
            20 => {
                crate::payload(request, 16)?;
                Ok(reply(0u64.to_le_bytes().to_vec()))
            }
            // GetDesiredLanguage -> u64 LanguageCode
            21 => {
                crate::payload(request, 0)?;
                Ok(reply(b"en-US\0\0\0".to_vec()))
            }
            // SetTerminateResult(u32): kept by the guest's own abort path.
            22 => {
                u32_argument(request)?;
                Ok(ServiceReply::default())
            }
            // GetDisplayVersion -> 0x10-byte string (the NACP is not parsed).
            23 => {
                crate::payload(request, 0)?;
                let mut version = vec![0u8; 16];
                version[..5].copy_from_slice(b"1.0.0");
                Ok(reply(version))
            }
            // NotifyRunning -> bool
            40 => {
                crate::payload(request, 0)?;
                Ok(reply(vec![1]))
            }
            // GetPseudoDeviceId -> 16 bytes, stable per install.
            50 => {
                crate::payload(request, 0)?;
                Ok(reply(vec![
                    0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x0f, 0xed, 0xcb, 0xa9, 0x87, 0x65, 0x43, 0x21,
                ]))
            }
            // GetGpuErrorDetectedSystemEvent (130),
            // GetFriendInvitationStorageChannelEvent (140),
            // GetNotificationStorageChannelEvent (150),
            // GetHealthWarningDisappearedSystemEvent (160): none of these
            // conditions can occur here, so the events are never signaled.
            130 | 140 | 150 | 160 => {
                crate::payload(request, 0)?;
                Ok(object(EventHandle::new(false)))
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IApplicationFunctions",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DebugFunctions;

impl Service for DebugFunctions {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        Err(ServiceError::UnsupportedCommand {
            service: "IDebugFunctions",
            command: request.command_id,
        })
    }
}

/// The main application proxy returned by OpenApplicationProxy (command 200) or
/// OpenLibraryAppletProxy (command 0). Also backwards-compatible with legacy AppletSession.
#[derive(Clone, Debug, Default)]
pub struct ApplicationProxy {
    state: SharedAppletState,
}

pub type AppletSession = ApplicationProxy;

impl Service for ApplicationProxy {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetCommonStateGetter (command 0) -> ICommonStateGetter
            0 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(CommonStateGetter { state: self.state.clone() })],
                effect: None,
            }),
            // GetSelfController (command 1) -> ISelfController. The reply has no
            // payload: any data here shifts where the guest reads the object ID.
            1 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SelfController::default())],
                effect: None,
            }),
            // GetWindowController (command 2) -> IWindowController
            2 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(WindowController::default())],
                effect: None,
            }),
            // GetAudioController (command 3) -> IAudioController
            3 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(AudioController::default())],
                effect: None,
            }),
            // GetDisplayController (command 4) -> IDisplayController
            4 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(DisplayController::default())],
                effect: None,
            }),
            // GetLibraryAppletCreator (command 11) -> ILibraryAppletCreator
            11 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(LibraryAppletCreator::default())],
                effect: None,
            }),
            // GetApplicationFunctions (command 20) -> IApplicationFunctions
            20 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(ApplicationFunctions { state: self.state.clone() })],
                effect: None,
            }),
            // Legacy GetOperationMode -> returns u8 (0 = Handheld, 1 = Docked)
            21 => {
                let mode = 0u8; // Handheld default
                Ok(ServiceReply {
                    result: 0,
                    data: vec![mode],
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // Legacy GetPerformanceMode -> returns u32 (0 = Handheld, 1 = Docked)
            22 => {
                let perf_mode = 0u32;
                Ok(ServiceReply {
                    result: 0,
                    data: perf_mode.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetDebugFunctions (command 1000) -> IDebugFunctions
            1000 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(DebugFunctions::default())],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IApplicationProxy",
                command,
            }),
        }
    }
}

impl Service for AppletManagerService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // OpenLibraryAppletProxy (0) / OpenApplicationProxy (200)
            0 | 200 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(ApplicationProxy { state: self.state.clone() })],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "appletAE",
                command,
            }),
        }
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &emulator_kernel::service::ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        // Proxy opens identify the caller with its own process handle.
        context.require_buffers(0, 0)?;
        if matches!(request.command_id, 0 | 200) {
            context.require_own_process(0)?;
        } else {
            context.require_copy_handles(0)?;
        }
        self.dispatch(request)
    }
}
