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

// Horizon HID shared memory layout (offsets into the 0x40000-byte block).
pub const DEBUG_PAD_OFFSET: usize = 0x0;
pub const TOUCH_SCREEN_OFFSET: usize = 0x400;
pub const MOUSE_OFFSET: usize = 0x3400;
pub const KEYBOARD_OFFSET: usize = 0x3800;
pub const NPAD_OFFSET: usize = 0x9a00;
pub const NPAD_ENTRY_SIZE: usize = 0x5000;
pub const NPAD_COUNT: usize = 10;

// Fields of one npad entry (`NpadInternalState`).
pub const NPAD_STYLE_SET: usize = 0x0;
pub const NPAD_JOY_ASSIGNMENT_MODE: usize = 0x4;
pub const NPAD_FULL_KEY_COLOR: usize = 0x8;
pub const NPAD_JOY_COLOR: usize = 0x14;
/// Seven common-state LIFOs (FullKey, Handheld, JoyDual, JoyLeft, JoyRight,
/// Palma, SystemExt), 0x350 bytes each.
pub const NPAD_COMMON_LIFOS: usize = 0x30;
pub const NPAD_COMMON_LIFO_SIZE: usize = 0x350;
/// Six six-axis LIFOs (FullKey, Handheld, JoyDual left/right, JoyLeft,
/// JoyRight), 0x708 bytes each.
pub const NPAD_SIX_AXIS_LIFOS: usize = 0x1760;
pub const NPAD_SIX_AXIS_LIFO_SIZE: usize = 0x708;
pub const NPAD_DEVICE_TYPE: usize = 0x4190;
pub const NPAD_SYSTEM_PROPERTIES: usize = 0x4198;
pub const NPAD_SYSTEM_BUTTON_PROPERTIES: usize = 0x41a0;
pub const NPAD_BATTERY_LEVELS: usize = 0x41a4;

/// LIFO header: u64 unused, u64 buffer count, u64 tail index, u64 count.
pub const LIFO_HEADER_SIZE: usize = 0x20;
pub const LIFO_ENTRIES: u64 = 17;

/// Npad style bits.
pub const NPAD_STYLE_FULL_KEY: u32 = 1 << 0;
pub const NPAD_STYLE_HANDHELD: u32 = 1 << 1;
/// Npad id of the console's own (handheld) controller and its entry index.
pub const NPAD_ID_HANDHELD: u32 = 0x20;
pub const HANDHELD_ENTRY: usize = 8;
/// Index of the Handheld LIFO among the common and six-axis LIFOs.
const HANDHELD_LIFO: usize = 1;

/// Common-state attributes: connected and wired, both halves attached.
pub const NPAD_ATTRIBUTES_HANDHELD: u32 = 0x3f;

/// Entry index in the npad array for an npad id (No1-No8, Handheld, Other).
pub fn npad_entry(id: u32) -> Option<usize> {
    match id {
        0..=7 => Some(id as usize),
        NPAD_ID_HANDHELD => Some(HANDHELD_ENTRY),
        0x10 => Some(9),
        _ => None,
    }
}

fn put(mem: &mut [u8], at: usize, bytes: &[u8]) {
    mem[at..at + bytes.len()].copy_from_slice(bytes);
}

/// A LIFO holding one sample (sampling number 0) whose state starts with
/// `state`; everything else in the sample is zero.
fn lifo(mem: &mut [u8], at: usize, state: &[u8]) {
    put(mem, at + 8, &LIFO_ENTRIES.to_le_bytes());
    put(mem, at + 0x10, &0u64.to_le_bytes()); // tail: entry 0
    put(mem, at + 0x18, &1u64.to_le_bytes()); // one sample
    // Entry 0: u64 sampling number, then the state.
    put(mem, at + LIFO_HEADER_SIZE + 8, state);
}

/// HID shared memory for a console whose only controller is the attached
/// handheld pair (npad 0x20, style Handheld) with nothing pressed. Every
/// other device and npad reads as disconnected. All LIFOs hold one sample;
/// sampling numbers do not advance until host input is wired in.
pub fn create_hid_shared_memory() -> Rc<RefCell<Vec<u8>>> {
    let mut mem = vec![0u8; HID_SHARED_MEMORY_SIZE];
    for device in [DEBUG_PAD_OFFSET, TOUCH_SCREEN_OFFSET, MOUSE_OFFSET, KEYBOARD_OFFSET] {
        lifo(&mut mem, device, &[]);
    }
    for entry in 0..NPAD_COUNT {
        let base = NPAD_OFFSET + entry * NPAD_ENTRY_SIZE;
        let handheld = entry == HANDHELD_ENTRY;
        for index in 0..7 {
            // NpadCommonState: sampling number, buttons, two sticks, attributes.
            let mut state = vec![0u8; 0x28];
            if handheld && index == HANDHELD_LIFO {
                state[0x20..0x24].copy_from_slice(&NPAD_ATTRIBUTES_HANDHELD.to_le_bytes());
            }
            lifo(&mut mem, base + NPAD_COMMON_LIFOS + index * NPAD_COMMON_LIFO_SIZE, &state);
        }
        for index in 0..6 {
            // SixAxisSensorState: delta time, sampling number, acceleration,
            // angular velocity, angle, 3x3 direction, attributes.
            let mut state = vec![0u8; 0x60];
            if handheld && index == HANDHELD_LIFO {
                state[0x18..0x1c].copy_from_slice(&(-1.0f32).to_le_bytes()); // gravity on z
                for axis in 0..3 {
                    let at = 0x34 + axis * 16; // identity direction matrix
                    state[at..at + 4].copy_from_slice(&1.0f32.to_le_bytes());
                }
                state[0x58..0x5c].copy_from_slice(&1u32.to_le_bytes()); // IsConnected
            }
            lifo(&mut mem, base + NPAD_SIX_AXIS_LIFOS + index * NPAD_SIX_AXIS_LIFO_SIZE, &state);
        }
        if !handheld {
            continue;
        }
        put(&mut mem, base + NPAD_STYLE_SET, &NPAD_STYLE_HANDHELD.to_le_bytes());
        put(&mut mem, base + NPAD_JOY_ASSIGNMENT_MODE, &0u32.to_le_bytes()); // Dual
        // Colors: attribute Ok (0), then main/sub RGBA for the pad and each half.
        for (at, words) in [
            (NPAD_FULL_KEY_COLOR, &[0u32, 0xff32_3232, 0xff0f_0f0f][..]),
            (NPAD_JOY_COLOR, &[0u32, 0xff82_8282, 0xff0f_0f0f, 0xff82_8282, 0xff0f_0f0f][..]),
        ] {
            for (i, word) in words.iter().enumerate() {
                put(&mut mem, base + at + i * 4, &word.to_le_bytes());
            }
        }
        // Device type: HandheldLeft | HandheldRight.
        put(&mut mem, base + NPAD_DEVICE_TYPE, &0x0cu32.to_le_bytes());
        // Powered left/right, ABXY oriented, plus/minus/d-pad available.
        let properties: u64 = (1 << 4) | (1 << 5) | (1 << 11) | (1 << 13) | (1 << 14) | (1 << 15);
        put(&mut mem, base + NPAD_SYSTEM_PROPERTIES, &properties.to_le_bytes());
        // Unintended home button input protection enabled.
        put(&mut mem, base + NPAD_SYSTEM_BUTTON_PROPERTIES, &1u32.to_le_bytes());
        for i in 0..3 {
            put(&mut mem, base + NPAD_BATTERY_LEVELS + i * 4, &4u32.to_le_bytes()); // full
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
