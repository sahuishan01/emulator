//! Original `hid` (Human Interface Device) service implementation.
//! Provides controller connection states, button inputs, touch screen events,
//! and six-axis motion sensor readings.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};
use emulator_kernel::Object;
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

#[derive(Clone, Debug)]
pub struct HidService {
    shared_memory: Rc<RefCell<Vec<u8>>>,
}

impl Default for HidService {
    fn default() -> Self {
        Self {
            shared_memory: create_hid_shared_memory(),
        }
    }
}

impl Service for HidService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // CreateAppletResource -> returns IAppletResource object
            0 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(HidAppletResource::new(self.shared_memory.clone()))],
                effect: None,
            }),
            // ActivateDebugPad
            1 => Ok(ServiceReply::default()),
            // SetSupportedNpadStyleSet
            100 => Ok(ServiceReply::default()),
            // GetSupportedNpadStyleSet
            101 => {
                let style_set = 0x1Fu32; // ProController | Handheld | JoyDual | JoyLeft | JoyRight
                Ok(ServiceReply {
                    result: 0,
                    data: style_set.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetSupportedNpadIdType
            102 => Ok(ServiceReply::default()),
            // ActivateNpad
            103 => Ok(ServiceReply::default()),
            // DeactivateNpad
            104 => Ok(ServiceReply::default()),
            // AcquireNpadStyleSetUpdateEventHandle
            106 => Ok(ServiceReply::default()),
            // SetNpadJoyAssignmentModeSingleByDefault
            120 => Ok(ServiceReply::default()),
            // SetNpadJoyAssignmentModeDual
            121 => Ok(ServiceReply::default()),
            // SetNpadHandheldActivationMode
            122 => Ok(ServiceReply::default()),
            // SetNpadJoyHoldType
            123 => Ok(ServiceReply::default()),
            // GetNpadJoyHoldType
            124 => Ok(ServiceReply {
                result: 0,
                data: 0u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // SetNpadJoyAssignmentModeSingle
            125 => Ok(ServiceReply::default()),
            // GetNpadJoyAssignmentMode
            126 => Ok(ServiceReply {
                result: 0,
                data: 0u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetActiveVibrationDeviceList
            200 => Ok(ServiceReply {
                result: 0,
                data: 0u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetVibrationDeviceInfo
            201 => Ok(ServiceReply {
                result: 0,
                data: vec![0u8; 8],
                moved_objects: Vec::new(),
                effect: None,
            }),
            // SendVibrationValue
            203 => Ok(ServiceReply::default()),
            // GetVibrationValue
            204 => Ok(ServiceReply {
                result: 0,
                data: vec![0u8; 16],
                moved_objects: Vec::new(),
                effect: None,
            }),
            // SetNpadCommunicationMode
            206 => Ok(ServiceReply::default()),
            // GetNpadCommunicationMode
            207 => Ok(ServiceReply {
                result: 0,
                data: 0u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "hid",
                command,
            }),
        }
    }
}
