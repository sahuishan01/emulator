//! Original, deliberately small service implementations for owned homebrew.
mod gpu;
mod nv;
mod nvas;
mod nvchannel;
mod nvmap;
pub mod applet;
pub mod fspsrv;
pub mod hid;
pub mod reference;
pub mod setsys;
pub mod timesys;
pub mod vi;
use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};
pub use applet::AppletManagerService;
pub use fspsrv::FspSrvService;
pub use gpu::DiagnosticGpu;
pub use hid::HidService;
pub use nv::NvDriver;
pub use setsys::SetSysService;
pub use timesys::TimeService;
pub use vi::ViManagerService;

fn payload(request: &Request, length: usize) -> Result<(), ServiceError> {
    if request.data.len() == length {
        Ok(())
    } else {
        Err(ServiceError::InvalidPayload {
            expected: length,
            actual: request.data.len(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ServiceManager {
    initialized: bool,
    graphics: Option<std::rc::Rc<dyn emulator_gpu::GraphicsBackend>>,
    romfs: Option<emulator_loader::RomFsArchive>,
}
impl ServiceManager {
    pub fn with_graphics(graphics: std::rc::Rc<dyn emulator_gpu::GraphicsBackend>) -> Self {
        Self {
            initialized: false,
            graphics: Some(graphics),
            romfs: None,
        }
    }

    pub fn with_romfs(romfs: emulator_loader::RomFsArchive) -> Self {
        Self {
            initialized: false,
            graphics: None,
            romfs: Some(romfs),
        }
    }

    pub fn set_romfs(&mut self, romfs: emulator_loader::RomFsArchive) {
        self.romfs = Some(romfs);
    }
}
impl Service for ServiceManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => {
                payload(request, 8)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                if self.initialized {
                    return Err(ServiceError::AlreadyInitialized);
                }
                self.initialized = true;
                Ok(ServiceReply::default())
            }
            1 => {
                payload(request, 8)?;
                if !self.initialized {
                    return Err(ServiceError::NotInitialized);
                }
                let name: [u8; 8] = request
                    .data
                    .as_slice()
                    .try_into()
                    .expect("validated payload");
                let object: Box<dyn Service> = match &name {
                    b"emu:test" => Box::new(DiagnosticCounter::default()),
                    b"emu:gpu\0" => Box::new(DiagnosticGpu::default()),
                    b"nvdrv\0\0\0" => Box::new(NvDriver::with_graphics(self.graphics.clone())),
                    b"fsp-srv\0" => Box::new(FspSrvService::with_romfs(self.romfs.clone())),
                    b"set:sys\0" => Box::new(SetSysService::default()),
                    b"time:s\0\0" | b"time:u\0\0" => Box::new(TimeService::default()),
                    b"vi:m\0\0\0\0" => Box::new(ViManagerService::default()),
                    b"appletAE" | b"appletOE" => Box::new(AppletManagerService::default()),
                    b"hid\0\0\0\0\0" => Box::new(HidService::default()),
                    _ => return Err(ServiceError::UnsupportedService(name)),
                };
                Ok(ServiceReply {
                    moved_objects: vec![object],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "sm:",
                command,
            }),
        }
    }
}

/// An explicitly emulator-owned service, not a Nintendo service-success stub.
#[derive(Clone, Debug, Default)]
pub struct DiagnosticCounter {
    value: u64,
}
impl Service for DiagnosticCounter {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => {
                payload(request, 8)?;
                let increment = u64::from_le_bytes(
                    request
                        .data
                        .as_slice()
                        .try_into()
                        .expect("validated payload"),
                );
                self.value = self
                    .value
                    .checked_add(increment)
                    .ok_or(ServiceError::ArithmeticOverflow)?;
            }
            1 => payload(request, 0)?,
            command => {
                return Err(ServiceError::UnsupportedCommand {
                    service: "emu:test",
                    command,
                })
            }
        }
        Ok(ServiceReply {
            data: self.value.to_le_bytes().to_vec(),
            ..ServiceReply::default()
        })
    }
}
