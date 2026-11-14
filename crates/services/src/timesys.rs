//! Original `time:s` / `time:u` / `time:a` (Time Service) implementation.
//! Provides steady clock, POSIX timestamps, system clock, and timezone interfaces for Switch runtimes.

use crate::event::EventHandle;
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

#[derive(Clone, Debug, Default)]
pub struct TimeZoneService;

impl Service for TimeZoneService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetDeviceLocationName: returns 0x24 bytes "UTC"
            0 => {
                let mut loc = vec![0u8; 0x24];
                loc[..3].copy_from_slice(b"UTC");
                Ok(ServiceReply {
                    result: 0,
                    data: loc,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // SetDeviceLocationName
            1 => Ok(ServiceReply::default()),
            // GetTotalLocationNameCount: returns 1u32
            2 => Ok(ServiceReply {
                result: 0,
                data: 1u32.to_le_bytes().to_vec(),
                moved_objects: Vec::new(),
                effect: None,
            }),
            // LoadLocationNameList: returns list of location names
            3 => {
                let mut list = vec![0u8; 0x24];
                list[..3].copy_from_slice(b"UTC");
                Ok(ServiceReply {
                    result: 0,
                    data: list,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // LoadTimeZoneRule: returns rule buffer
            4 => {
                let rule = vec![0u8; 0x400];
                Ok(ServiceReply {
                    result: 0,
                    data: rule,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // ToCalendarTime: calendar conversion stub
            100 => {
                let mut calendar = vec![0u8; 0x10];
                calendar[0..2].copy_from_slice(&2026u16.to_le_bytes()); // year
                calendar[2] = 1; // month
                calendar[3] = 1; // day
                Ok(ServiceReply {
                    result: 0,
                    data: calendar,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "ITimeZoneService",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TimeService;

impl Service for TimeService {
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
            // CalculateUserLocalSystemClockContext / CalculateSpanBetween
            1 | 2 => {
                let mut ctx = vec![0u8; 0x20];
                ctx[..8].copy_from_slice(&1767225600u64.to_le_bytes());
                Ok(ServiceReply {
                    result: 0,
                    data: ctx,
                    moved_objects: Vec::new(),
                    effect: None,
                })
            }
            // GetStandardUserSystemClock -> ISystemClock
            3 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SystemClockService::default())],
                effect: None,
            }),
            // GetStandardNetworkSystemClock -> ISystemClock
            4 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SystemClockService::default())],
                effect: None,
            }),
            // GetStandardSteadyClock -> ISteadyClock
            5 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(SteadyClockService::default())],
                effect: None,
            }),
            // GetTimeZoneService -> ITimeZoneService
            6 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(TimeZoneService::default())],
                effect: None,
            }),
            // GetSharedMemoryNativeHandle (command 20)
            20 => Ok(ServiceReply {
                result: 0,
                data: Vec::new(),
                moved_objects: vec![Box::new(EventHandle::new(true))],
                effect: None,
            }),
            command => Err(ServiceError::UnsupportedCommand {
                service: "time",
                command,
            }),
        }
    }
}
