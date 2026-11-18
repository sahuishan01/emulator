//! Original `hid` (Human Interface Device) service implementation.
//! Provides controller connection states, button inputs, touch screen events,
//! and six-axis motion sensor readings.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};
use emulator_kernel::Object;
use crate::event::EventHandle;
use std::cell::RefCell;
use std::rc::Rc;

pub const HID_SHARED_MEMORY_SIZE: usize = 0x40000; // 256 KiB

pub fn create_hid_shared_memory() -> Rc<RefCell<Vec<u8>>> {
    let mut mem = vec![0u8; HID_SHARED_MEMORY_SIZE];
    // TouchScreen at 0x400: Header
    // header: sample counter (u64), latest index (u64)
    mem[0x400..0x408].copy_from_slice(&1u64.to_le_bytes());
    mem[0x408..0x410].copy_from_slice(&0u64.to_le_bytes());

    // Npad entries at 0x9A00 + i * 0x5000:
    // Npad 0: Handheld, Npad 1: Player1, etc.
    for i in 0..10 {
        let base = 0x9A00 + i * 0x5000;
        if base + 0x5000 <= HID_SHARED_MEMORY_SIZE {
            // Header at base:
            // sample counter = 17, latest sample index = 0, sample count = 17
            mem[base..base + 8].copy_from_slice(&17u64.to_le_bytes());
            mem[base + 8..base + 16].copy_from_slice(&0u64.to_le_bytes());
            mem[base + 16..base + 24].copy_from_slice(&17u64.to_le_bytes());

            // Initialize 17 samples at base + 0x20
            let sample_size = 0x40; // 64 bytes per sample
            for s in 0..17 {
                let sample_offset = base + 0x20 + s * sample_size;
                // sampling_number: u64
                mem[sample_offset..sample_offset + 8].copy_from_slice(&((s + 1) as u64).to_le_bytes());
                // sampling_number2: u64
                mem[sample_offset + 8..sample_offset + 16].copy_from_slice(&((s + 1) as u64).to_le_bytes());
                // buttons: u64 (0 = no buttons)
                mem[sample_offset + 16..sample_offset + 24].copy_from_slice(&0u64.to_le_bytes());
                // analog_stick_l: x: i32, y: i32 (centered = 0, 0)
                mem[sample_offset + 24..sample_offset + 28].copy_from_slice(&0i32.to_le_bytes());
                mem[sample_offset + 28..sample_offset + 32].copy_from_slice(&0i32.to_le_bytes());
                // analog_stick_r: x: i32, y: i32 (centered = 0, 0)
                mem[sample_offset + 32..sample_offset + 36].copy_from_slice(&0i32.to_le_bytes());
                mem[sample_offset + 36..sample_offset + 40].copy_from_slice(&0i32.to_le_bytes());
                // attributes: u64 (1 = connected, 2 = wired)
                let attr: u64 = if i <= 1 { 1 } else { 0 };
                mem[sample_offset + 40..sample_offset + 48].copy_from_slice(&attr.to_le_bytes());
            }
        }
    }
    Rc::new(RefCell::new(mem))
}

#[derive(Clone, Debug)]
pub struct SharedMemoryHandle(pub Rc<RefCell<Vec<u8>>>);

impl Service for SharedMemoryHandle {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, _request: &Request) -> Result<ServiceReply, ServiceError> {
        Ok(ServiceReply::default())
    }
    fn as_handle(&self) -> Option<Object> {
        Some(Object::SharedMemory(self.0.clone()))
    }
}

#[derive(Clone, Debug)]
pub struct HidAppletResource {
    shared_memory: Rc<RefCell<Vec<u8>>>,
}

impl Default for HidAppletResource {
    fn default() -> Self {
        Self {
            shared_memory: create_hid_shared_memory(),
        }
    }
}

impl HidAppletResource {
    pub fn new(shared_memory: Rc<RefCell<Vec<u8>>>) -> Self {
        Self { shared_memory }
    }
}

impl Service for HidAppletResource {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetSharedMemoryHandle (command 0): provides shared memory handle for input state polling
            0 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SharedMemoryHandle(self.shared_memory.clone()))],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IAppletResource",
                command,
            }),
        }
    }
}

/// Npad configuration the guest sets and reads back through `hid`.
#[derive(Clone, Debug)]
struct NpadConfig {
    style_set: u32,
    joy_hold_type: u64,
    handheld_activation_mode: u64,
    communication_mode: u64,
    vibration_permitted: bool,
    /// SetTouchScreenResolution: the guest's touch coordinate space.
    touch_resolution: (u32, u32),
}

impl Default for NpadConfig {
    fn default() -> Self {
        Self {
            style_set: 0x1f, // FullKey | Handheld | JoyDual | JoyLeft | JoyRight
            joy_hold_type: 1, // Horizontal
            handheld_activation_mode: 0,
            communication_mode: 0,
            vibration_permitted: true,
            touch_resolution: (1280, 720),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HidService {
    shared_memory: Rc<RefCell<Vec<u8>>>,
    /// Per-request copy (clone_for_request), committed by the kernel on success.
    config: NpadConfig,
}

impl Default for HidService {
    fn default() -> Self {
        Self {
            shared_memory: create_hid_shared_memory(),
            config: NpadConfig::default(),
        }
    }
}

impl HidService {
    /// Touch coordinate space the guest asked for (default 1280x720); host
    /// touch input is scaled into it.
    pub fn touch_resolution(&self) -> (u32, u32) {
        self.config.touch_resolution
    }
}

fn data(bytes: Vec<u8>) -> Result<ServiceReply, ServiceError> {
    Ok(ServiceReply {
        result: 0,
        data: bytes,
        moved_objects: Vec::new(),
        effect: None,
    })
}

fn object(object: Box<dyn Service>) -> Result<ServiceReply, ServiceError> {
    Ok(ServiceReply {
        result: 0,
        data: Vec::new(),
        moved_objects: vec![object],
        effect: None,
    })
}

fn input_u32(request: &Request) -> Result<u32, ServiceError> {
    request
        .data
        .get(..4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
        .ok_or(ServiceError::InvalidPayload {
            expected: 4,
            actual: request.data.len(),
        })
}

/// IActiveVibrationDeviceList: only ActivateVibrationDevice (0).
#[derive(Clone, Debug, Default)]
pub struct ActiveVibrationDeviceList;

impl Service for ActiveVibrationDeviceList {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            0 => Ok(ServiceReply::default()),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IActiveVibrationDeviceList",
                command,
            }),
        }
    }
}

impl Service for HidService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    /// IHidServer. Input devices are emulated as one always-connected handheld
    /// controller; configuration setters are stored and read back.
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        let config = &mut self.config;
        match request.command_id {
            // CreateAppletResource -> IAppletResource
            0 => object(Box::new(HidAppletResource::new(self.shared_memory.clone()))),
            // Activate{DebugPad,TouchScreen,Mouse,Keyboard,Gesture}
            1 | 11 | 21 | 31 | 91 => Ok(ServiceReply::default()),
            // Start/StopSixAxisSensor, EnableSixAxisSensorFusion,
            // Set/ResetSixAxisSensorFusionParameters, SetGyroscopeZeroDriftMode
            66 | 67 | 69 | 70 | 72 | 79 => Ok(ServiceReply::default()),
            // IsSixAxisSensorFusionEnabled
            68 => data(vec![1]),
            // IsSixAxisSensorAtRest
            82 => data(vec![1]),
            // SetSupportedNpadStyleSet
            100 => {
                config.style_set = input_u32(request)?;
                Ok(ServiceReply::default())
            }
            // GetSupportedNpadStyleSet
            101 => data(config.style_set.to_le_bytes().to_vec()),
            // ActivateNpad, DeactivateNpad, DisconnectNpad, ActivateNpadWithRevision
            // (SetSupportedNpadIdType, 102, carries a buffer: see `prepare`)
            103 | 104 | 107 | 109 => Ok(ServiceReply::default()),
            // AcquireNpadStyleSetUpdateEventHandle: signalled once (controller attached)
            106 => object(Box::new(EventHandle::new(true))),
            // GetPlayerLedPattern
            108 => data(1u64.to_le_bytes().to_vec()),
            // SetNpadJoyHoldType
            120 => {
                config.joy_hold_type = u64::from(input_u32(request)?);
                Ok(ServiceReply::default())
            }
            // GetNpadJoyHoldType
            121 => data(config.joy_hold_type.to_le_bytes().to_vec()),
            // SetNpadJoyAssignmentMode{SingleByDefault,Single,Dual}, MergeSingleJoyAsDualJoy,
            // Start/StopLrAssignmentMode
            122..=127 => Ok(ServiceReply::default()),
            // SetNpadHandheldActivationMode
            128 => {
                config.handheld_activation_mode = u64::from(input_u32(request)?);
                Ok(ServiceReply::default())
            }
            // GetNpadHandheldActivationMode
            129 => data(config.handheld_activation_mode.to_le_bytes().to_vec()),
            // SwapNpadAssignment
            130 => Ok(ServiceReply::default()),
            // IsUnintendedHomeButtonInputProtectionEnabled
            131 => data(vec![1]),
            // EnableUnintendedHomeButtonInputProtection, SetNpadAnalogStickUseCenterClamp,
            // SetNpadCaptureButtonAssignment, ClearNpadCaptureButtonAssignment
            132 | 134 | 135 | 136 => Ok(ServiceReply::default()),
            // GetVibrationDeviceInfo: linear resonant actuator, left/right by handle bit
            200 => {
                let handle = input_u32(request)?;
                let position = if handle >> 24 & 1 == 0 { 1u32 } else { 2 };
                let mut info = 1u32.to_le_bytes().to_vec();
                info.extend_from_slice(&position.to_le_bytes());
                data(info)
            }
            // SendVibrationValue, SendVibrationValues, SendVibrationGmvValue,
            // Begin/EndPermitVibrationSession
            201 | 206 | 207 | 209 | 210 => Ok(ServiceReply::default()),
            // GetActualVibrationValue: neutral amplitudes at the default frequencies
            202 => {
                let mut value = Vec::with_capacity(16);
                for field in [0.0f32, 160.0, 0.0, 320.0] {
                    value.extend_from_slice(&field.to_le_bytes());
                }
                data(value)
            }
            // CreateActiveVibrationDeviceList
            203 => object(Box::new(ActiveVibrationDeviceList)),
            // PermitVibration
            204 => {
                config.vibration_permitted = request.data.first().is_some_and(|&b| b != 0);
                Ok(ServiceReply::default())
            }
            // IsVibrationPermitted
            205 => data(vec![u8::from(config.vibration_permitted)]),
            // IsVibrationDeviceMounted
            211 => data(vec![1]),
            // Activate/Start/StopConsoleSixAxisSensor, Activate/StartSevenSixAxisSensor
            300..=304 => Ok(ServiceReply::default()),
            // SetNpadCommunicationMode
            1000 => {
                config.communication_mode = u64::from(input_u32(request)?);
                Ok(ServiceReply::default())
            }
            // GetNpadCommunicationMode
            1001 => data(config.communication_mode.to_le_bytes().to_vec()),
            // SetTouchScreenConfiguration
            1002 => Ok(ServiceReply::default()),
            // SetTouchScreenResolution(u32 width, u32 height, u64 ARUID, pid)
            1004 => {
                crate::payload(request, 16)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                let width = u32::from_le_bytes(request.data[0..4].try_into().expect("validated"));
                let height = u32::from_le_bytes(request.data[4..8].try_into().expect("validated"));
                if width == 0 || height == 0 {
                    return Err(ServiceError::DeviceFault(format!("touch screen resolution {width}x{height}")));
                }
                config.touch_resolution = (width, height);
                Ok(ServiceReply::default())
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "hid",
                command,
            }),
        }
    }

    fn prepare(
        &mut self,
        request: &Request,
        context: &ServiceContext<'_>,
    ) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        if request.command_id != 102 {
            context.require_buffers(0, 0)?;
            return self.dispatch(request);
        }
        // SetSupportedNpadIdType(u64 ARUID, X buffer: array of u32 npad ids)
        if request.data.len() != 8 {
            return Err(ServiceError::InvalidPayload {
                expected: 8,
                actual: request.data.len(),
            });
        }
        let ids = context.pointer_send(0)?;
        let length = ids.as_ref().map_or(0, |view| view.len());
        if length % 4 != 0 {
            return Err(ServiceError::InvalidPayload {
                expected: length & !3,
                actual: length,
            });
        }
        Ok(ServiceReply::default())
    }
}
