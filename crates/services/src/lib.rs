//! Original, deliberately small service implementations for owned homebrew.
mod gpu;
mod nv;
pub use nv::nv_failures;
mod nvas;
mod nvchannel;
mod nvmap;
pub mod acc;
pub mod aoc;
pub mod apm;
pub mod ectx;
pub mod applet;
pub mod aud;
pub mod bcat;
pub mod csrng;
pub mod binder;
pub mod event;
pub mod fspsrv;
pub mod hid;
pub mod hwopus;
pub mod lm;
pub mod pctl;
pub mod plu;
pub mod reference;
pub mod savedata;
pub mod setsys;
pub mod timesys;
pub mod timezone;
pub mod vi;
use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};
pub use acc::AccountService;
pub use apm::ApmService;
pub use applet::AppletManagerService;
pub use aud::{AudioOutManager, AudioRendererManager};
pub use event::EventHandle;
pub use fspsrv::FspSrvService;
pub use gpu::DiagnosticGpu;
pub use hid::HidService;
pub use nv::NvDriver;
pub use setsys::SetSysService;
pub use timesys::TimeService;
pub use vi::ViManagerService;

/// Deferred write of reply bytes into a pinned guest receive buffer.
#[derive(Debug)]
pub(crate) struct BufferWrite {
    pub(crate) view: emulator_memory::MemoryView,
    pub(crate) bytes: Vec<u8>,
    pub(crate) ranges: Vec<emulator_memory::BackingRange>,
}

impl BufferWrite {
    pub(crate) fn new(view: emulator_memory::MemoryView, bytes: Vec<u8>) -> Self {
        let ranges = view.backing_ranges().collect();
        Self { view, bytes, ranges }
    }
}

impl emulator_kernel::service::PreparedServiceEffect for BufferWrite {
    fn write_ranges(&self) -> &[emulator_memory::BackingRange] {
        &self.ranges
    }
    fn commit(self: Box<Self>) {
        self.view
            .write(0, &self.bytes)
            .expect("prepared output pin");
    }
}

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
    /// Raw data storage of the current program (OpenDataStorageByCurrentProcess).
    data_storage: Option<DataStorage>,
    /// Applet messages and launch parameters, shared by every applet session.
    applet: applet::SharedAppletState,
    /// Committed save data, shared by every fsp-srv session.
    save_data: savedata::SaveDataStore,
}

/// Shared read-only storage handed to every fsp-srv session.
pub type DataStorage = std::rc::Rc<dyn emulator_loader::storage::ReadOnlyStorage>;

impl ServiceManager {
    pub fn with_graphics(graphics: std::rc::Rc<dyn emulator_gpu::GraphicsBackend>) -> Self {
        Self {
            graphics: Some(graphics),
            ..Self::default()
        }
    }

    /// In-memory RomFS: serves both the RomFS filesystem and the raw data storage.
    pub fn with_romfs(romfs: emulator_loader::RomFsArchive) -> Self {
        let mut manager = Self::default();
        manager.set_romfs(romfs);
        manager
    }

    pub fn set_romfs(&mut self, romfs: emulator_loader::RomFsArchive) {
        self.data_storage = Some(std::rc::Rc::new(romfs.clone()));
        self.romfs = Some(romfs);
    }

    /// Raw data storage only (large RomFS read lazily from the container).
    pub fn set_data_storage(&mut self, storage: DataStorage) {
        self.data_storage = Some(storage);
    }
}
impl Service for ServiceManager {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => {
                // CMIF carries an 8-byte PID placeholder; TIPC sends only the
                // kernel-filled process ID descriptor.
                payload(request, if request.tipc { 0 } else { 8 })?;
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
                    b"nvdrv\0\0\0" | b"nvdrv:a\0" | b"nvdrv:s\0" | b"nvdrv:t\0" => {
                        Box::new(NvDriver::with_graphics(self.graphics.clone()))
                    }
                    b"fsp-srv\0" | b"fsp-pr\0\0" => Box::new(
                        FspSrvService::with_storage(self.romfs.clone(), self.data_storage.clone())
                            .with_save_data(self.save_data.clone()),
                    ),
                    b"set:sys\0" | b"set\0\0\0\0\0" => Box::new(SetSysService::default()),
                    b"time:s\0\0" | b"time:u\0\0" | b"time:a\0\0" => Box::new(TimeService::default()),
                    b"vi:m\0\0\0\0" | b"vi:u\0\0\0\0" | b"vi:s\0\0\0\0" => {
                        Box::new(ViManagerService::with_graphics(self.graphics.clone()))
                    }
                    b"appletAE" | b"appletOE" => Box::new(AppletManagerService::with_state(self.applet.clone())),
                    b"hid\0\0\0\0\0" | b"hid:sys\0" | b"hid:dbg\0" => Box::new(HidService::default()),
                    b"acc:u0\0\0" | b"acc:u1\0\0" | b"acc:e\0\0\0" => {
                        Box::new(AccountService::default())
                    }
                    b"apm\0\0\0\0\0" | b"apm:p\0\0\0" | b"apm:sys\0" => Box::new(ApmService::default()),
                    b"audren:u" => Box::new(AudioRendererManager::default()),
                    b"lm\0\0\0\0\0\0" => Box::new(lm::LogManager),
                    b"aoc:u\0\0\0" => Box::new(aoc::AddOnContentManager),
                    b"ectx:aw\0" => Box::new(ectx::ErrorContextWriter),
                    b"pctl\0\0\0\0" | b"pctl:s\0\0" | b"pctl:a\0\0" | b"pctl:r\0\0" => {
                        Box::new(pctl::ParentalControlFactory)
                    }
                    b"audout:u" => Box::new(AudioOutManager::default()),
                    b"hwopus\0\0" => Box::new(hwopus::OpusDecoderManager),
                    b"bcat:u\0\0" => Box::new(bcat::BcatServiceCreator),
                    b"csrng\0\0\0" => Box::new(csrng::RandomService),
                    b"pl:u\0\0\0\0" => Box::new(plu::SharedFontService::default()),
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
