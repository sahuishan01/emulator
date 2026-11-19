//! Original `vi:m` (Visual Interface) service implementation.
//! Provides display creation, layer management, and buffer presentation endpoints.

use crate::binder::{BinderError, BufferQueue, QueuedFrame};
use emulator_ipc::Request;
use emulator_kernel::service::PreparedServiceEffect;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use emulator_kernel::TimerQueue;
use emulator_memory::BackingRange;
use std::cell::Cell;
use std::rc::Rc;

/// Display refresh period: 60 Hz.
pub const VSYNC_PERIOD_NS: u64 = 16_666_667;

#[derive(Clone, Debug, Default)]
pub struct ViManagerService {
    graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>,
    /// The default display's vsync signal, shared by its display services.
    vsync: Rc<Cell<bool>>,
}

impl ViManagerService {
    pub fn with_graphics(graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>) -> Self {
        Self {
            graphics,
            vsync: Rc::default(),
        }
    }
}

/// Arms the periodic vsync timer the first time the event is handed out.
#[derive(Debug)]
struct ArmVsync {
    timers: Rc<TimerQueue>,
    signal: Rc<Cell<bool>>,
    first_ns: u64,
}

impl PreparedServiceEffect for ArmVsync {
    fn write_ranges(&self) -> &[BackingRange] {
        &[]
    }
    fn complete(&mut self) -> Result<(), ServiceError> {
        if !self.timers.is_armed(&self.signal) {
            self.timers
                .arm_periodic(&self.signal, self.first_ns, VSYNC_PERIOD_NS)
                .map_err(|error| ServiceError::DeviceFault(format!("vsync timer: {error:?}")))?;
        }
        Ok(())
    }
    fn commit(self: Box<Self>) {}
}

/// `IHOSBinderDriver`: routes parcels to each layer's buffer queue.
#[derive(Clone, Debug)]
pub struct HosBinderDriver {
    queues: Vec<(u32, BufferQueue)>,
    graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>,
}

impl HosBinderDriver {
    pub fn with_graphics(graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>) -> Self {
        let queue = || BufferQueue::new(DISPLAY_WIDTH as u32, DISPLAY_HEIGHT as u32);
        Self {
            queues: vec![(LAYER_BINDER_ID, queue()), (RECORDING_BINDER_ID, queue())],
            graphics,
        }
    }

    pub fn queue(&self, binder_id: u32) -> Option<&BufferQueue> {
        self.queues
            .iter()
            .find(|(id, _)| *id == binder_id)
            .map(|(_, q)| q)
    }

    /// TransactParcel (0, A/B buffers) and TransactParcelAuto (3, auto-select).
    fn transact(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        if !(12..=16).contains(&request.data.len()) {
            crate::payload(request, 12)?;
        }
        let (input, output) = if request.command_id == 0 {
            context.require_buffers(1, 1)?;
            (context.send_buffer(0)?, context.receive_buffer(0)?)
        } else {
            context.require_auto_buffers(1, 1)?;
            match (context.auto_send(0)?, context.auto_receive(0)?) {
                (Some(input), Some(output)) => (input, output),
                _ => return Ok(error_reply(RESULT_INVALID_ARGUMENT)),
            }
        };
        let word =
            |at: usize| u32::from_le_bytes(request.data[at..at + 4].try_into().expect("checked"));
        let (binder_id, code) = (word(0), word(4));
        let Some(queue) = self
            .queues
            .iter_mut()
            .find(|(id, _)| *id == binder_id)
            .map(|(_, q)| q)
        else {
            return Ok(error_reply(RESULT_INVALID_ARGUMENT));
        };
        let mut parcel = vec![0; input.len()];
        input.read(0, &mut parcel).map_err(ServiceError::Memory)?;
        let graphics = self.graphics.clone();
        let reply = queue.transact(code, &parcel, |frame| {
            if let Some(backend) = &graphics {
                present_placeholder(backend.as_ref(), frame);
            }
        });
        let reply = match reply {
            Ok(reply) => reply,
            Err(BinderError::UnsupportedTransaction(code)) => {
                return Err(ServiceError::UnsupportedCommand {
                    service: "IGraphicBufferProducer",
                    command: code,
                })
            }
            Err(_) => return Ok(error_reply(RESULT_INVALID_ARGUMENT)),
        };
        if reply.len() > output.len() {
            return Ok(error_reply(RESULT_INVALID_ARGUMENT));
        }
        Ok(ServiceReply {
            effect: Some(Box::new(crate::BufferWrite::new(output, reply))),
            ..ServiceReply::default()
        })
    }
}

/// vi `InvalidArgument` (module 114, description 1).
const RESULT_INVALID_ARGUMENT: u32 = 0x272;

fn error_reply(result: u32) -> ServiceReply {
    ServiceReply {
        result,
        ..ServiceReply::default()
    }
}

/// Queued buffer contents are not decoded yet (GPU surfaces are not
/// resolved); the backend receives a cleared frame of the buffer's size.
fn present_placeholder(backend: &dyn emulator_gpu::GraphicsBackend, frame: &QueuedFrame) {
    let (width, height) = (frame.buffer.width, frame.buffer.height);
    if width == 0 || height == 0 {
        return;
    }
    let background = emulator_gpu::ColorClear {
        address: emulator_gpu::GpuVa(0),
        width,
        height,
        pitch: frame.buffer.stride.max(width) * 4,
        format: frame.buffer.format,
        rect: [0, 0, width, height],
        color: [0; 4],
        mask: 15,
    };
    let _ = backend.render(&emulator_gpu::GraphicsFrame {
        background,
        draws: Vec::new(),
        pixels: vec![0u8; width as usize * height as usize * 4],
    });
}

impl Default for HosBinderDriver {
    fn default() -> Self {
        Self::with_graphics(None)
    }
}

impl Service for HosBinderDriver {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn name(&self) -> &'static str {
        "IHOSBinderDriver"
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 | 3 => self.transact(request, context),
            _ => {
                context.require_buffers(0, 0)?;
                context.require_copy_handles(0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 | 3 => Err(ServiceError::MissingMemoryContext),
            // AdjustRefcount(id, delta, type): references are not tracked.
            1 => {
                crate::payload(request, 12)?;
                Ok(ServiceReply::default())
            }
            // GetNativeHandle(id, type): the buffer-available event. Slots are
            // released at queue time, so a buffer is always available.
            2 => {
                crate::payload(request, 8)?;
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(crate::event::EventHandle::new(true))],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IHOSBinderDriver",
                command,
            }),
        }
    }
}

/// Binder object ID of the application layer's IGraphicBufferProducer.
pub const LAYER_BINDER_ID: u32 = 1;
/// Binder object ID of the recording layer's IGraphicBufferProducer.
pub const RECORDING_BINDER_ID: u32 = 2;
/// ID of the presentable application layer.
pub const DEFAULT_LAYER_ID: u64 = 1;
/// ID of the recording layer from CreateManagedDisplaySeparableLayer.
pub const RECORDING_LAYER_ID: u64 = 2;
/// ID returned for the "Default" display.
pub const DEFAULT_DISPLAY_ID: u64 = 0;
const DISPLAY_WIDTH: u64 = 1280;
const DISPLAY_HEIGHT: u64 = 720;

/// Serialized parcel describing the layer's native window: a 16-byte parcel
/// header followed by a flattened binder reference to the buffer producer.
pub fn native_window_parcel(binder_id: u32) -> Vec<u8> {
    let mut object = Vec::with_capacity(0x28);
    object.extend_from_slice(&2u32.to_le_bytes()); // object magic
    object.extend_from_slice(&1u32.to_le_bytes()); // owning process
    object.extend_from_slice(&binder_id.to_le_bytes());
    object.extend_from_slice(&[0; 12]);
    object.extend_from_slice(b"dispdrv\0");
    object.extend_from_slice(&[0; 8]);
    let mut parcel = Vec::with_capacity(0x10 + object.len() + 4);
    parcel.extend_from_slice(&(object.len() as u32).to_le_bytes()); // data size
    parcel.extend_from_slice(&0x10u32.to_le_bytes()); // data offset
                                                      // The guest's Parcel::readObject only accepts a flat object whose data
                                                      // position is listed in the objects table, so list offset 0.
    parcel.extend_from_slice(&4u32.to_le_bytes()); // objects size
    parcel.extend_from_slice(&(0x10 + object.len() as u32).to_le_bytes()); // objects offset
    parcel.extend_from_slice(&object);
    parcel.extend_from_slice(&0u32.to_le_bytes()); // objects[0]: data offset of the binder
    parcel
}

fn words(values: &[u64]) -> ServiceReply {
    ServiceReply {
        result: 0,
        data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        moved_objects: Vec::new(),
        effect: None,
    }
}

fn read_u64(request: &Request, offset: usize) -> u64 {
    u64::from_le_bytes(
        request.data[offset..offset + 8]
            .try_into()
            .expect("length checked"),
    )
}

/// Accepts only the "Default" display name (NUL-padded, 0x40 bytes).
fn default_display_name(name: &[u8]) -> bool {
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    &name[..end] == b"Default"
}

/// `IApplicationDisplayService`, shared by vi:u / vi:s / vi:m.
#[derive(Clone, Debug, Default)]
pub struct DisplayService {
    graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>,
    vsync: Rc<Cell<bool>>,
}

impl DisplayService {
    pub fn with_graphics(graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>) -> Self {
        Self {
            graphics,
            vsync: Rc::default(),
        }
    }

    /// GetDisplayVsyncEvent(u64 display_id): the default display's vsync
    /// event, signaled every [`VSYNC_PERIOD_NS`] of guest time from the
    /// first request on.
    fn vsync_event(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_buffers(0, 0)?;
        context.require_copy_handles(0)?;
        crate::payload(request, 8)?;
        if read_u64(request, 0) != DEFAULT_DISPLAY_ID {
            return Err(ServiceError::InvalidPayload {
                expected: 8,
                actual: request.data.len(),
            });
        }
        Ok(ServiceReply {
            moved_objects: vec![Box::new(crate::event::EventHandle(self.vsync.clone()))],
            effect: Some(Box::new(ArmVsync {
                timers: context.timers().clone(),
                signal: self.vsync.clone(),
                first_ns: context.virtual_time_ns().saturating_add(VSYNC_PERIOD_NS),
            })),
            ..ServiceReply::default()
        })
    }

    /// Commands writing a B (receive) buffer: the parcel or display list.
    fn with_output(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        context.require_buffers(0, 1)?;
        let output = context.receive_buffer(0)?;
        let (reply, bytes) = match request.command_id {
            // ListDisplays() -> u64 count, B: DisplayInfo[]
            1000 => {
                crate::payload(request, 0)?;
                let mut info = vec![0u8; 0x60];
                info[..7].copy_from_slice(b"Default");
                info[0x40] = 1; // has limited layers
                info[0x48..0x50].copy_from_slice(&1u64.to_le_bytes()); // max layers
                info[0x50..0x58].copy_from_slice(&DISPLAY_WIDTH.to_le_bytes());
                info[0x58..0x60].copy_from_slice(&DISPLAY_HEIGHT.to_le_bytes());
                (words(&[1]), info)
            }
            // OpenLayer(DisplayName, u64 layer_id, u64 aruid) -> u64 size, B: parcel
            2020 => {
                crate::payload(request, 0x50)?;
                let layer = read_u64(request, 0x40);
                if !default_display_name(&request.data[..0x40])
                    || !matches!(layer, DEFAULT_LAYER_ID | RECORDING_LAYER_ID)
                {
                    return Err(ServiceError::InvalidPayload {
                        expected: 0x50,
                        actual: request.data.len(),
                    });
                }
                // Each layer gets its own binder object, numbered like the layer.
                let parcel = native_window_parcel(layer as u32);
                (words(&[parcel.len() as u64]), parcel)
            }
            // CreateStrayLayer(u32 flags, u64 display_id) -> u64 layer_id, u64 size, B: parcel
            2030 => {
                crate::payload(request, 16)?;
                if read_u64(request, 8) != DEFAULT_DISPLAY_ID {
                    return Err(ServiceError::InvalidPayload {
                        expected: 16,
                        actual: request.data.len(),
                    });
                }
                let parcel = native_window_parcel(LAYER_BINDER_ID);
                (words(&[DEFAULT_LAYER_ID, parcel.len() as u64]), parcel)
            }
            _ => unreachable!("routed by prepare"),
        };
        if bytes.len() > output.len() {
            return Err(ServiceError::InvalidPayload {
                expected: bytes.len(),
                actual: output.len(),
            });
        }
        let mut reply = reply;
        reply.effect = Some(Box::new(crate::BufferWrite::new(output, bytes)));
        Ok(reply)
    }
}

impl Service for DisplayService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn name(&self) -> &'static str {
        "IApplicationDisplayService"
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            1000 | 2020 | 2030 => self.with_output(request, context),
            5202 => self.vsync_event(request, context),
            _ => {
                context.require_buffers(0, 0)?;
                context.require_copy_handles(0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetRelayService / GetIndirectDisplayTransactionService -> IHOSBinderDriver
            100 | 103 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(HosBinderDriver::with_graphics(
                        self.graphics.clone(),
                    ))],
                    effect: None,
                })
            }
            // GetSystemDisplayService
            101 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(SystemDisplayService)],
                    ..ServiceReply::default()
                })
            }
            // GetManagerDisplayService
            102 => {
                crate::payload(request, 0)?;
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(ManagerDisplayService)],
                    ..ServiceReply::default()
                })
            }
            // OpenDisplay(DisplayName) -> u64 display_id
            1010 => {
                crate::payload(request, 0x40)?;
                if !default_display_name(&request.data) {
                    return Err(ServiceError::InvalidPayload {
                        expected: 0x40,
                        actual: request.data.len(),
                    });
                }
                Ok(words(&[DEFAULT_DISPLAY_ID]))
            }
            // OpenDefaultDisplay() -> u64 display_id
            1011 => {
                crate::payload(request, 0)?;
                Ok(words(&[DEFAULT_DISPLAY_ID]))
            }
            // CloseDisplay(u64)
            1020 => {
                crate::payload(request, 8)?;
                Ok(ServiceReply::default())
            }
            // GetDisplayResolution(u64) -> u64 width, u64 height
            1102 => {
                crate::payload(request, 8)?;
                Ok(words(&[DISPLAY_WIDTH, DISPLAY_HEIGHT]))
            }
            // CloseLayer(u64) / DestroyStrayLayer(u64)
            2021 | 2031 => {
                crate::payload(request, 8)?;
                Ok(ServiceReply::default())
            }
            // SetLayerScalingMode(u32 mode, u64 layer_id)
            2101 => {
                crate::payload(request, 16)?;
                Ok(ServiceReply::default())
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IApplicationDisplayService",
                command,
            }),
        }
    }
}

/// `ISystemDisplayService`: no commands are implemented yet.
#[derive(Clone, Debug, Default)]
pub struct SystemDisplayService;

impl Service for SystemDisplayService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn name(&self) -> &'static str {
        "ISystemDisplayService"
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        Err(ServiceError::UnsupportedCommand {
            service: "ISystemDisplayService",
            command: request.command_id,
        })
    }
}

/// `IManagerDisplayService`: no commands are implemented yet.
#[derive(Clone, Debug, Default)]
pub struct ManagerDisplayService;

impl Service for ManagerDisplayService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn name(&self) -> &'static str {
        "IManagerDisplayService"
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        Err(ServiceError::UnsupportedCommand {
            service: "IManagerDisplayService",
            command: request.command_id,
        })
    }
}

impl Service for ViManagerService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetDisplayService (e.g. 0 = Application display service)
            0 | 1 | 2 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(DisplayService {
                    graphics: self.graphics.clone(),
                    vsync: self.vsync.clone(),
                })],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "vi:m",
                command,
            }),
        }
    }
}
