//! Original `vi:m` (Visual Interface) service implementation.
//! Provides display creation, layer management, and buffer presentation endpoints.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferState {
    Free,
    Dequeued,
    Queued,
    Acquired,
}

#[derive(Clone, Debug)]
pub struct BufferSlot {
    pub state: BufferState,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub usage: u32,
}

impl Default for BufferSlot {
    fn default() -> Self {
        Self {
            state: BufferState::Free,
            width: 1280,
            height: 720,
            stride: 1280,
            format: 1, // RGBA_8888
            usage: 0xb00,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ViManagerService {
    graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>,
}

impl ViManagerService {
    pub fn with_graphics(graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>) -> Self {
        Self { graphics }
    }
}

#[derive(Clone, Debug)]
pub struct HosBinderDriver {
    connected: bool,
    connected_api: i32,
    current_slot: usize,
    num_pending: u32,
    slots: Vec<BufferSlot>,
    graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>,
}

impl HosBinderDriver {
    pub fn with_graphics(graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>) -> Self {
        Self {
            connected: false,
            connected_api: 0,
            current_slot: 0,
            num_pending: 0,
            slots: vec![BufferSlot::default(); 3],
            graphics,
        }
    }
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

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // TransactParcel (0) & TransactParcelAuto (3)
            0 | 3 => {
                let binder_code = if request.data.len() >= 8 {
                    u32::from_le_bytes(request.data[4..8].try_into().unwrap_or([0; 4]))
                } else {
                    0
                };
                let mut reply_data = Vec::new();
                match binder_code {
                    // 1: REQUEST_BUFFER
                    1 => {
                        let slot_idx = if request.data.len() >= 12 {
                            u32::from_le_bytes(request.data[8..12].try_into().unwrap_or([0; 4])) as usize
                        } else {
                            0
                        };
                        let slot = self.slots.get(slot_idx).cloned().unwrap_or_default();
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&1u32.to_le_bytes()); // buffer present
                        reply_data.extend_from_slice(&slot.width.to_le_bytes()); // width
                        reply_data.extend_from_slice(&slot.height.to_le_bytes());  // height
                        reply_data.extend_from_slice(&slot.stride.to_le_bytes()); // stride
                        reply_data.extend_from_slice(&slot.format.to_le_bytes()); // format
                        reply_data.extend_from_slice(&slot.usage.to_le_bytes()); // usage
                    }
                    // 2: SET_BUFFER_COUNT
                    2 => {
                        let count = if request.data.len() >= 12 {
                            u32::from_le_bytes(request.data[8..12].try_into().unwrap_or([3; 4])) as usize
                        } else {
                            3
                        };
                        let count = count.clamp(1, 64);
                        self.slots.resize(count, BufferSlot::default());
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 3: DEQUEUE_BUFFER
                    3 => {
                        let slot_count = self.slots.len();
                        let mut chosen = None;
                        for i in 0..slot_count {
                            let idx = (self.current_slot + i) % slot_count;
                            if self.slots[idx].state == BufferState::Free {
                                chosen = Some(idx);
                                break;
                            }
                        }
                        let slot = chosen.unwrap_or(self.current_slot % slot_count);
                        self.slots[slot].state = BufferState::Dequeued;
                        self.current_slot = (slot + 1) % slot_count;

                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&(slot as u32).to_le_bytes()); // slot
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // has_fence = 0
                    }
                    // 4: DETACH_BUFFER
                    4 => {
                        let slot_idx = if request.data.len() >= 12 {
                            u32::from_le_bytes(request.data[8..12].try_into().unwrap_or([0; 4])) as usize
                        } else {
                            0
                        };
                        if let Some(slot) = self.slots.get_mut(slot_idx) {
                            slot.state = BufferState::Free;
                        }
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 5: ATTACH_BUFFER
                    5 => {
                        let slot_count = self.slots.len();
                        let mut chosen = None;
                        for i in 0..slot_count {
                            let idx = (self.current_slot + i) % slot_count;
                            if self.slots[idx].state == BufferState::Free {
                                chosen = Some(idx);
                                break;
                            }
                        }
                        let slot = chosen.unwrap_or(self.current_slot % slot_count);
                        self.slots[slot].state = BufferState::Dequeued;
                        self.current_slot = (slot + 1) % slot_count;
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                        reply_data.extend_from_slice(&(slot as u32).to_le_bytes());
                    }
                    // 7: QUEUE_BUFFER
                    7 => {
                        let slot_idx = if request.data.len() >= 12 {
                            u32::from_le_bytes(request.data[8..12].try_into().unwrap_or([0; 4])) as usize
                        } else {
                            0
                        };
                        if let Some(slot) = self.slots.get_mut(slot_idx) {
                            slot.state = BufferState::Queued;
                        }
                        self.num_pending = self.num_pending.saturating_add(1);

                        if let Some(backend) = &self.graphics {
                            let slot = self.slots.get(slot_idx).cloned().unwrap_or_default();
                            let bg = emulator_gpu::ColorClear {
                                address: emulator_gpu::GpuVa(0),
                                width: slot.width,
                                height: slot.height,
                                pitch: slot.stride * 4,
                                format: slot.format,
                                rect: [0, 0, slot.width, slot.height],
                                color: [0; 4],
                                mask: 15,
                            };
                            let frame = emulator_gpu::GraphicsFrame {
                                background: bg,
                                draws: Vec::new(),
                                pixels: vec![0u8; (slot.width * slot.height * 4) as usize],
                            };
                            let _ = backend.render(&frame);
                        }

                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&1280u32.to_le_bytes()); // width
                        reply_data.extend_from_slice(&720u32.to_le_bytes());  // height
                        reply_data.extend_from_slice(&0u32.to_le_bytes());    // transform
                        reply_data.extend_from_slice(&self.num_pending.to_le_bytes()); // pending buffers
                    }
                    // 8: CANCEL_BUFFER
                    8 => {
                        let slot_idx = if request.data.len() >= 12 {
                            u32::from_le_bytes(request.data[8..12].try_into().unwrap_or([0; 4])) as usize
                        } else {
                            0
                        };
                        if let Some(slot) = self.slots.get_mut(slot_idx) {
                            slot.state = BufferState::Free;
                        }
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 9: QUERY
                    9 => {
                        let what = if request.data.len() >= 12 {
                            u32::from_le_bytes(request.data[8..12].try_into().unwrap_or([0; 4]))
                        } else {
                            0
                        };
                        let val = match what {
                            1 => 1280, // NATIVE_WINDOW_WIDTH
                            2 => 720,  // NATIVE_WINDOW_HEIGHT
                            3 => 1,    // NATIVE_WINDOW_FORMAT (RGBA_8888)
                            4 | 6 => 1, // NATIVE_WINDOW_MIN_UNDEQUEUED_BUFFERS
                            7 => self.num_pending, // NATIVE_WINDOW_QUEUED_BUFFERS
                            8 => 1280, // NATIVE_WINDOW_DEFAULT_WIDTH
                            9 => 720,  // NATIVE_WINDOW_DEFAULT_HEIGHT
                            10 => 0,   // NATIVE_WINDOW_TRANSFORM_HINT
                            _ => 0,
                        };
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&val.to_le_bytes());
                    }
                    // 10: CONNECT
                    10 => {
                        self.connected = true;
                        self.connected_api = if request.data.len() >= 12 {
                            i32::from_le_bytes(request.data[8..12].try_into().unwrap_or([0; 4]))
                        } else {
                            1
                        };
                        reply_data.extend_from_slice(&0u32.to_le_bytes()); // status OK
                        reply_data.extend_from_slice(&1280u32.to_le_bytes());
                        reply_data.extend_from_slice(&720u32.to_le_bytes());
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 11: DISCONNECT
                    11 => {
                        self.connected = false;
                        self.connected_api = 0;
                        self.num_pending = 0;
                        for slot in &mut self.slots {
                            slot.state = BufferState::Free;
                        }
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 14: SET_PRE_ALLOCATED_BUFFER
                    14 => {
                        let slot_idx = if request.data.len() >= 12 {
                            u32::from_le_bytes(request.data[8..12].try_into().unwrap_or([0; 4])) as usize
                        } else {
                            0
                        };
                        if let Some(slot) = self.slots.get_mut(slot_idx) {
                            if request.data.len() >= 28 {
                                slot.width = u32::from_le_bytes(request.data[12..16].try_into().unwrap_or(1280u32.to_le_bytes()));
                                slot.height = u32::from_le_bytes(request.data[16..20].try_into().unwrap_or(720u32.to_le_bytes()));
                                slot.stride = u32::from_le_bytes(request.data[20..24].try_into().unwrap_or(1280u32.to_le_bytes()));
                                slot.format = u32::from_le_bytes(request.data[24..28].try_into().unwrap_or(1u32.to_le_bytes()));
                            }
                            slot.state = BufferState::Free;
                        }
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    // 16: GET_BUFFER_HISTORY
                    16 => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                    _ => {
                        reply_data.extend_from_slice(&0u32.to_le_bytes());
                    }
                }
                Ok(ServiceReply {
                    result: 0,
                    data: reply_data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // AdjustRefcount (1)
            1 => Ok(ServiceReply::default()),
            // GetNativeHandle (2)
            2 => Ok(ServiceReply::default()),
            command => Err(ServiceError::UnsupportedCommand {
                service: "IHOSBinderDriver",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DisplayService {
    graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>,
}

impl DisplayService {
    pub fn with_graphics(graphics: Option<Rc<dyn emulator_gpu::GraphicsBackend>>) -> Self {
        Self { graphics }
    }
}

impl Service for DisplayService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetRelayService (command 100), GetSystemDisplayService (101), GetManagerDisplayService (102), GetIndirectDisplayService (103)
            100 | 101 | 102 | 103 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(HosBinderDriver::with_graphics(self.graphics.clone()))],
                    effect: None,
                })
            }
            // ListDisplays (command 1000)
            1000 => {
                let mut data = vec![0u8; 64];
                data[..7].copy_from_slice(b"Default");
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // OpenDisplay (command 1010) -> returns DisplayId (u64)
            1010 => {
                let display_id = 0u64;
                Ok(ServiceReply {
                    result: 0,
                    data: display_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // CloseDisplay (command 1011)
            1011 => Ok(ServiceReply::default()),
            // GetNaturalDisplayResolution (command 1020) -> returns width=1280 (u32), height=720 (u32)
            1020 => {
                let mut data = Vec::with_capacity(8);
                data.extend_from_slice(&1280u32.to_le_bytes());
                data.extend_from_slice(&720u32.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetDisplayMagnification (command 1101)
            1101 => Ok(ServiceReply::default()),
            // SetDisplayAlpha (command 1102)
            1102 => Ok(ServiceReply::default()),
            // CreateStrayLayer (command 2020) -> returns LayerId (u64), IGraphicBufferProducer parcel
            2020 => {
                let layer_id = 1u64;
                let mut data = Vec::with_capacity(16);
                data.extend_from_slice(&layer_id.to_le_bytes());
                data.extend_from_slice(&0u64.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // DestroyStrayLayer (command 2030)
            2030 => Ok(ServiceReply::default()),
            // CreateIndirectLayer (command 2040)
            2040 => {
                let layer_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: layer_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // DestroyIndirectLayer (command 2041)
            2041 => Ok(ServiceReply::default()),
            // CreateManagedLayer (command 2050) -> returns LayerId (u64)
            2050 => {
                let layer_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: layer_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // DestroyManagedLayer (command 2060)
            2060 => Ok(ServiceReply::default()),
            // SetLayerScalingMode (command 2101)
            2101 => Ok(ServiceReply::default()),
            // ConvertScalingMode (command 2102)
            2102 => Ok(ServiceReply::default()),
            // SetLayerZOrder (command 2201)
            2201 => Ok(ServiceReply::default()),
            // SetLayerPosition (command 2203)
            2203 => Ok(ServiceReply::default()),
            // SetLayerSize (command 2204)
            2204 => Ok(ServiceReply::default()),
            // GetLayerAlpha (command 2205) -> returns float alpha (1.0)
            2205 => {
                Ok(ServiceReply {
                    result: 0,
                    data: 1.0f32.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetLayerAlpha (command 2207)
            2207 => Ok(ServiceReply::default()),
            // SetLayerCrop (command 2209)
            2209 => Ok(ServiceReply::default()),
            // CreateSharedLayer (command 5201)
            5201 => {
                let layer_id = 1u64;
                Ok(ServiceReply {
                    result: 0,
                    data: layer_id.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // OpenSharedLayer (command 5202)
            5202 => {
                let data = vec![0u8; 16];
                Ok(ServiceReply {
                    result: 0,
                    data,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // CloseSharedLayer (command 5203)
            5203 => Ok(ServiceReply::default()),
            // AddToStereoPair (command 6000), RemoveFromStereoPair (6001), SetStereoPairLayerPosition (6002)
            6000 | 6001 | 6002 => Ok(ServiceReply::default()),
            // GetDisplayVsyncEvent (command 8225)
            8225 => {
                // Returns 0 status for vsync event subscription
                Ok(ServiceReply::default())
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "IApplicationDisplayService",
                command,
            }),
        }
    }
}

impl Service for ViManagerService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetDisplayService (e.g. 0 = Application display service)
            0 | 1 | 2 => {
                Ok(ServiceReply {
                    result: 0,
                    data: Vec::new(),
                    moved_objects: vec![Box::new(DisplayService::with_graphics(self.graphics.clone()))],
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "vi:m",
                command,
            }),
        }
    }
}
