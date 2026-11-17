//! Original `time:s` / `time:u` / `time:a` (Time Service) implementation.
//! Provides steady clock, POSIX timestamps, system clock, and timezone interfaces for Switch runtimes.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

#[derive(Clone, Debug, Default)]
pub struct SystemClockService;

impl Service for SystemClockService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetCurrentTime: returns POSIX seconds as u64
            0 => {
                let timestamp = 1767225600u64;
                Ok(ServiceReply {
                    result: 0,
                    data: timestamp.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetCurrentTime
            1 => Ok(ServiceReply::default()),
            // GetSystemClockContext: returns 0x20 bytes context
            2 => {
                let mut ctx = vec![0u8; 0x20];
                ctx[..8].copy_from_slice(&1767225600u64.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data: ctx,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetSystemClockContext
            3 => Ok(ServiceReply::default()),
            command => Err(ServiceError::UnsupportedCommand {
                service: "ISystemClock",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SteadyClockService;

impl Service for SteadyClockService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetCurrentTimePoint: returns 0x18 bytes steady time point
            0 => {
                let mut pt = vec![0u8; 0x18];
                pt[..8].copy_from_slice(&1000u64.to_le_bytes()); // ticks / elapsed seconds
                Ok(ServiceReply {
                    result: 0,
                    data: pt,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetTestOffset: returns 0i64
            1 => Ok(ServiceReply {
                result: 0,
                data: 0i64.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // SetTestOffset
            2 => Ok(ServiceReply::default()),
            // GetRtcValue: returns u64 seconds
            3 => {
                let rtc = 1767225600u64;
                Ok(ServiceReply {
                    result: 0,
                    data: rtc.to_le_bytes().to_vec(),
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // IsRtcResetDetected: returns 0u8
            4 => Ok(ServiceReply {
                result: 0,
                data: vec![0u8],
                moved_objects: Vec::new(),
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "ISteadyClock",
                command,
            }),
        }
    }
}

pub use crate::timezone::TimeZoneService;

#[derive(Clone, Debug, Default)]
pub struct TimeService;

impl Service for TimeService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // IStaticService command table (time:u / time:a / time:s).
            // GetStandardUserSystemClock / GetStandardNetworkSystemClock /
            // GetStandardLocalSystemClock / GetEphemeralNetworkSystemClock
            0 | 1 | 4 | 5 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SystemClockService::default())],
                effect: None,
            }),
            // GetStandardSteadyClock -> ISteadyClock
            2 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SteadyClockService::default())],
                effect: None,
            }),
            // GetTimeZoneService -> ITimeZoneService
            3 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(TimeZoneService::default())],
                effect: None,
            }),
            // IsStandardUserSystemClockAutomaticCorrectionEnabled: false
            100 => Ok(ServiceReply {
                result: 0,
                data: vec![0],
                moved_objects: Vec::new(),
                effect: None,
            }),
            // GetSharedMemoryNativeHandle (command 20)
            20 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(crate::hid::SharedMemoryHandle(
                    time_shared_memory(),
                ))],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "time",
                command,
            }),
        }
    }
}

/// Fixed identity of the emulated steady clock source.
const CLOCK_SOURCE_ID: [u8; 16] = *b"emu-steady-clock";

/// Time shared memory page. Each entry is a lock-free pair: a u32 update
/// counter followed (8-byte aligned) by two copies of the value, of which
/// copy `counter & 1` is current. Counters stay 0, so copy 0 is filled.
///
/// - 0x00 steady clock: base time point (ns) + clock source ID.
/// - 0x38 local and 0x80 network system clock contexts: offset (s) +
///   steady time point (s) + clock source ID.
/// - 0xc8 automatic correction flag (off), 0xd0 continuous adjustment (unset).
///
/// The guest clock is deterministic: the same fixed epoch as `ISystemClock`
/// plus emulated uptime, never host time.
fn time_shared_memory() -> std::rc::Rc<std::cell::RefCell<Vec<u8>>> {
    let mut page = vec![0_u8; 0x1000];
    page[0x10..0x20].copy_from_slice(&CLOCK_SOURCE_ID);
    for context in [0x38, 0x80] {
        page[context + 8..context + 16].copy_from_slice(&1767225600_u64.to_le_bytes());
        page[context + 24..context + 40].copy_from_slice(&CLOCK_SOURCE_ID);
    }
    std::rc::Rc::new(std::cell::RefCell::new(page))
}
