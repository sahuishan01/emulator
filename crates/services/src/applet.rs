//! Original `appletOE` / `appletAE` (Applet Manager) service implementation.
//! Manages game execution lifecycle, focus handling, operation mode, and system controllers.

use crate::event::EventHandle;
use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct AppletManagerService;

#[derive(Clone, Debug, Default)]
pub struct CommonStateGetter;

impl Service for CommonStateGetter {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetEventHandle: event signaled on state/focus changes
            0 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(EventHandle::new(true))],
                effect: None,
            }),
            // ReceiveMessage: returns message code (0x0F = FocusStateChanged)
            1 => {
                let msg = 0x0Fu32;
                Ok(ServiceReply {
                    result: 0,
                    data: msg.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
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
            // CreateManagedDisplaySeparableLayer: returns LayerId (u64)
            41 => {
                let layer_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: layer_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetHandlesRequestToDisplay, SetAlbumImageOrientation
            50 | 62 => Ok(ServiceReply::default()),
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

    fn dispatch(&mut self, _request: &Request) -> Result<ServiceReply, ServiceError> {
        Ok(ServiceReply::default())
    }
}

#[derive(Clone, Debug, Default)]
pub struct LibraryAppletCreator;

impl Service for LibraryAppletCreator {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, _request: &Request) -> Result<ServiceReply, ServiceError> {
        Ok(ServiceReply::default())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ApplicationFunctions;

impl Service for ApplicationFunctions {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // InitializeApplicationCopyrightFlag
            1 => Ok(ServiceReply::default()),
            // EnsureSaveData: returns required size (0)
            20 => {
                let size = 0u64;
                Ok(ServiceReply {
                    result: 0,
                    data: size.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetDesiredLanguage: returns LanguageCode ("en-US")
            21 => {
                let lang = b"en-US\0\0\0";
                Ok(ServiceReply {
                    result: 0,
                    data: lang.to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetTerminateResult
            22 => Ok(ServiceReply::default()),
            // GetDisplayVersion: returns 0x10 bytes version string ("1.0.0")
            23 => {
                let mut ver = vec![0u8; 16];
                ver[..5].copy_from_slice(b"1.0.0");
                Ok(ServiceReply {
                    result: 0,
                    data: ver,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // NotifyRunning: returns bool (1)
            40 => Ok(ServiceReply {
                result: 0,
                data: vec![1u8],
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetPseudoDeviceId: returns 16-byte pseudo UUID
            50 => {
                let pseudo_id = [
                    0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                    0x0f, 0xed, 0xcb, 0xa9, 0x87, 0x65, 0x43, 0x21,
                ];
                Ok(ServiceReply {
                    result: 0,
                    data: pseudo_id.to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // InitializeGamePlayRecording, SetGamePlayRecordingState
            66 | 67 => Ok(ServiceReply::default()),
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

    fn dispatch(&mut self, _request: &Request) -> Result<ServiceReply, ServiceError> {
        Ok(ServiceReply::default())
    }
}

/// The main application proxy returned by OpenApplicationProxy (command 200) or
/// OpenLibraryAppletProxy (command 0). Also backwards-compatible with legacy AppletSession.
#[derive(Clone, Debug, Default)]
pub struct ApplicationProxy;

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
                moved_objects: vec![Box::new(CommonStateGetter::default())],
                effect: None,
            }),
            // GetSelfController (command 1) -> ISelfController
            1 if request.data.is_empty() => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SelfController::default())],
                effect: None,
            }),
            // Legacy GetAppletResourceUserId (command 1 with non-empty or fallback)
            1 => {
                let user_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: user_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
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
                moved_objects: vec![Box::new(ApplicationFunctions::default())],
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
                moved_objects: vec![Box::new(ApplicationProxy::default())],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "appletAE",
                command,
            }),
        }
    }
}
