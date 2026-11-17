//! Original `ITimeZoneService` with the project's own zone data.
//!
//! No console time zone archive is used. The service knows fixed-offset zones
//! only (`UTC`, `GMT`, `Etc/UTC`, `Etc/GMT`, `Etc/GMT+N`, `Etc/GMT-N`). Any
//! other location name fails with TimeZoneNotFound instead of being treated as
//! UTC. The device location is `UTC`.
//!
//! `TimeZoneRule` is opaque to guests (they pass it back to the conversion
//! commands), so the 0x4000-byte rule uses a project-defined layout:
//! `"EMTZ"`, version 1, UTC offset in seconds (i32), 8-byte abbreviation,
//! zero padding.

use crate::BufferWrite;
use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceContext, ServiceError, ServiceReply};

/// Size of a `LocationName` (NUL-padded ASCII).
pub const LOCATION_NAME_SIZE: usize = 0x24;
/// Size of a `TimeZoneRule` buffer.
pub const TIME_ZONE_RULE_SIZE: usize = 0x4000;
const RULE_MAGIC: &[u8; 4] = b"EMTZ";
const RULE_VERSION: u32 = 1;
/// Identifies this project's zone data, not an IANA tzdata release.
const RULE_DATA_VERSION: &[u8] = b"emu-fixed-1";

const fn time_result(description: u32) -> u32 {
    116 | (description << 9)
}
/// Calendar or POSIX time outside the representable range.
pub const RESULT_TIME_OUT_OF_RANGE: u32 = time_result(902);
/// The rule buffer is not a rule this service produced.
pub const RESULT_TIME_ZONE_CONVERSION_FAILED: u32 = time_result(903);
/// The location name is not a zone this service knows.
pub const RESULT_TIME_ZONE_NOT_FOUND: u32 = time_result(989);

/// A fixed-offset zone: canonical name, offset east of UTC, abbreviation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    pub name: String,
    pub offset: i32,
    pub abbreviation: String,
}

/// Every location name the service lists, in list order.
pub fn zone_names() -> Vec<String> {
    let mut names: Vec<String> = ["UTC", "GMT", "Etc/UTC", "Etc/GMT"].map(String::from).to_vec();
    names.extend((1..=12).map(|hours| format!("Etc/GMT+{hours}")));
    names.extend((1..=14).map(|hours| format!("Etc/GMT-{hours}")));
    names
}

/// Looks up a location name. `Etc/GMT+N` is N hours west (POSIX sign).
pub fn find_zone(name: &str) -> Option<Zone> {
    let zone = |offset: i32, abbreviation: String| Zone { name: name.to_string(), offset, abbreviation };
    match name {
        "UTC" | "Etc/UTC" => return Some(zone(0, "UTC".into())),
        "GMT" | "Etc/GMT" => return Some(zone(0, "GMT".into())),
        _ => {}
    }
    let rest = name.strip_prefix("Etc/GMT")?;
    let (west, digits) = match rest.as_bytes().first()? {
        b'+' => (true, &rest[1..]),
        b'-' => (false, &rest[1..]),
        _ => return None,
    };
    if digits.is_empty() || digits.len() > 2 || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits.parse().ok()?;
    if (west && hours > 12) || (!west && hours > 14) {
        return None;
    }
    let offset = if west { -hours * 3600 } else { hours * 3600 };
    Some(zone(offset, format!("{}{hours:02}", if west { '-' } else { '+' })))
}

fn location_name(bytes: &[u8]) -> Result<String, ServiceError> {
    let raw = bytes.get(..LOCATION_NAME_SIZE).ok_or(ServiceError::InvalidPayload {
        expected: LOCATION_NAME_SIZE,
        actual: bytes.len(),
    })?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
}

fn encode_name(name: &str) -> [u8; LOCATION_NAME_SIZE] {
    let mut out = [0u8; LOCATION_NAME_SIZE];
    let bytes = name.as_bytes();
    let length = bytes.len().min(LOCATION_NAME_SIZE - 1);
    out[..length].copy_from_slice(&bytes[..length]);
    out
}

/// Serialises a zone as this service's `TimeZoneRule`.
pub fn encode_rule(zone: &Zone) -> Vec<u8> {
    let mut rule = vec![0u8; TIME_ZONE_RULE_SIZE];
    rule[..4].copy_from_slice(RULE_MAGIC);
    rule[4..8].copy_from_slice(&RULE_VERSION.to_le_bytes());
    rule[8..12].copy_from_slice(&zone.offset.to_le_bytes());
    let abbreviation = zone.abbreviation.as_bytes();
    rule[12..12 + abbreviation.len().min(7)].copy_from_slice(&abbreviation[..abbreviation.len().min(7)]);
    rule
}

/// Reads back a rule written by [`encode_rule`]: (offset, abbreviation).
pub fn decode_rule(rule: &[u8]) -> Option<(i32, [u8; 8])> {
    if rule.len() < 20 || &rule[..4] != RULE_MAGIC || rule[4..8] != RULE_VERSION.to_le_bytes() {
        return None;
    }
    let offset = i32::from_le_bytes(rule[8..12].try_into().ok()?);
    let mut abbreviation = [0u8; 8];
    abbreviation.copy_from_slice(&rule[12..20]);
    Some((offset, abbreviation))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (month 1..=12).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12; // March = 0
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`]: (year, month 1..=12, day 1..=31).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `CalendarTime` (8 bytes) plus `CalendarAdditionalInfo` (0x18 bytes) for a
/// POSIX time in a zone, or `None` when the year does not fit an i16.
pub fn to_calendar(posix: i64, offset: i32, abbreviation: [u8; 8]) -> Option<Vec<u8>> {
    let local = posix.checked_add(i64::from(offset))?;
    let days = local.div_euclid(86_400);
    let seconds = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let year = i16::try_from(year).ok()?;
    let mut out = vec![0u8; 0x20];
    out[0..2].copy_from_slice(&year.to_le_bytes());
    out[2] = month as u8;
    out[3] = day as u8;
    out[4] = (seconds / 3600) as u8;
    out[5] = (seconds / 60 % 60) as u8;
    out[6] = (seconds % 60) as u8;
    let day_of_week = (days + 4).rem_euclid(7) as u32; // 1970-01-01 was a Thursday
    let day_of_year = (days - days_from_civil(i64::from(year), 1, 1)) as u32;
    out[8..12].copy_from_slice(&day_of_week.to_le_bytes());
    out[12..16].copy_from_slice(&day_of_year.to_le_bytes());
    out[16..24].copy_from_slice(&abbreviation);
    out[24..28].copy_from_slice(&0u32.to_le_bytes()); // fixed zones have no DST
    out[28..32].copy_from_slice(&offset.to_le_bytes());
    Some(out)
}

/// POSIX time of a `CalendarTime` in a zone. Out-of-range fields carry over
/// (month 13 is January of the next year), as with C `mktime`.
pub fn to_posix(calendar: &[u8], offset: i32) -> Option<i64> {
    let year = i64::from(i16::from_le_bytes(calendar.get(0..2)?.try_into().ok()?));
    let [month, day, hour, minute, second] =
        [2, 3, 4, 5, 6].map(|at| i64::from(calendar[at] as i8));
    let months = year * 12 + (month - 1);
    let days = days_from_civil(months.div_euclid(12), months.rem_euclid(12) + 1, 1) + day - 1;
    let local = days * 86_400 + hour * 3600 + minute * 60 + second;
    local.checked_sub(i64::from(offset))
}

fn device_zone() -> Zone {
    find_zone("UTC").expect("UTC is built in")
}

fn word(value: u32) -> ServiceReply {
    ServiceReply { data: value.to_le_bytes().to_vec(), ..ServiceReply::default() }
}

fn failure(result: u32) -> ServiceReply {
    ServiceReply { result, ..ServiceReply::default() }
}

fn payload(request: &Request, length: usize) -> Result<&[u8], ServiceError> {
    request.data.get(..length).ok_or(ServiceError::InvalidPayload { expected: length, actual: request.data.len() })
}

fn posix_argument(request: &Request) -> Result<i64, ServiceError> {
    Ok(i64::from_le_bytes(payload(request, 8)?.try_into().expect("8 bytes")))
}

/// Writes `bytes` to the start of a receive buffer as a deferred effect.
fn output(view: emulator_memory::MemoryView, bytes: Vec<u8>, data: Vec<u8>) -> Result<ServiceReply, ServiceError> {
    if view.len() < bytes.len() {
        return Err(ServiceError::InvalidPayload { expected: bytes.len(), actual: view.len() });
    }
    let view = view.slice(0, bytes.len()).map_err(ServiceError::Memory)?;
    Ok(ServiceReply {
        data,
        effect: Some(Box::new(BufferWrite::new(view, bytes))),
        ..ServiceReply::default()
    })
}

fn read_rule(context: &ServiceContext<'_>) -> Result<Option<(i32, [u8; 8])>, ServiceError> {
    let view = context.send_buffer(0)?;
    if view.len() < TIME_ZONE_RULE_SIZE {
        return Ok(None);
    }
    let mut header = [0u8; 20];
    view.read(0, &mut header).map_err(ServiceError::Memory)?;
    Ok(decode_rule(&header))
}

#[derive(Clone, Debug, Default)]
pub struct TimeZoneService;

impl Service for TimeZoneService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }

    fn prepare(&mut self, request: &Request, context: &ServiceContext<'_>) -> Result<ServiceReply, ServiceError> {
        context.require_copy_handles(0)?;
        match request.command_id {
            // LoadLocationNameList(u32 index) -> u32 count, B: LocationName[]
            3 => {
                context.require_buffers(0, 1)?;
                let index = u32::from_le_bytes(payload(request, 4)?.try_into().expect("4 bytes")) as usize;
                let view = context.receive_buffer(0)?;
                let capacity = view.len() / LOCATION_NAME_SIZE;
                let names: Vec<String> = zone_names().into_iter().skip(index).take(capacity).collect();
                let bytes: Vec<u8> = names.iter().flat_map(|name| encode_name(name)).collect();
                let count = (names.len() as u32).to_le_bytes().to_vec();
                if bytes.is_empty() {
                    return Ok(ServiceReply { data: count, ..ServiceReply::default() });
                }
                output(view, bytes, count)
            }
            // LoadTimeZoneRule(LocationName) -> B: TimeZoneRule
            4 => {
                context.require_buffers(0, 1)?;
                let name = location_name(&request.data)?;
                let Some(zone) = find_zone(&name) else {
                    return Ok(failure(RESULT_TIME_ZONE_NOT_FOUND));
                };
                output(context.receive_buffer(0)?, encode_rule(&zone), Vec::new())
            }
            // ToCalendarTime(PosixTime, A: TimeZoneRule) -> CalendarTime, CalendarAdditionalInfo
            100 => {
                context.require_buffers(1, 0)?;
                let posix = posix_argument(request)?;
                let Some((offset, abbreviation)) = read_rule(context)? else {
                    return Ok(failure(RESULT_TIME_ZONE_CONVERSION_FAILED));
                };
                Ok(to_calendar(posix, offset, abbreviation).map_or_else(
                    || failure(RESULT_TIME_OUT_OF_RANGE),
                    |data| ServiceReply { data, ..ServiceReply::default() },
                ))
            }
            // ToPosixTime(CalendarTime, A: TimeZoneRule) -> u32 count, C: PosixTime[]
            201 | 202 => {
                let rule = if request.command_id == 201 {
                    if context.send_buffer(0).is_err() {
                        return Err(ServiceError::InvalidBufferCounts { send: 0, receive: 0 });
                    }
                    let Some((offset, _)) = read_rule(context)? else {
                        return Ok(failure(RESULT_TIME_ZONE_CONVERSION_FAILED));
                    };
                    offset
                } else {
                    device_zone().offset
                };
                let calendar = payload(request, 8)?;
                let Some(view) = context.pointer_receive(0)?.filter(|view| view.len() >= 8) else {
                    return Err(ServiceError::InvalidPayload { expected: 8, actual: 0 });
                };
                let Some(posix) = to_posix(calendar, rule) else {
                    return Ok(failure(RESULT_TIME_OUT_OF_RANGE));
                };
                output(view, posix.to_le_bytes().to_vec(), 1u32.to_le_bytes().to_vec())
            }
            _ => {
                context.require_buffers(0, 0)?;
                self.dispatch(request)
            }
        }
    }

    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // GetDeviceLocationName -> LocationName
            0 => Ok(ServiceReply { data: encode_name(&device_zone().name).to_vec(), ..ServiceReply::default() }),
            // GetTotalLocationNameCount -> u32
            2 => Ok(word(zone_names().len() as u32)),
            // GetTimeZoneRuleVersion -> 0x10-byte version string
            5 => {
                let mut version = vec![0u8; 0x10];
                version[..RULE_DATA_VERSION.len()].copy_from_slice(RULE_DATA_VERSION);
                Ok(ServiceReply { data: version, ..ServiceReply::default() })
            }
            // ToCalendarTimeWithMyRule(PosixTime) -> CalendarTime, CalendarAdditionalInfo
            101 => {
                let zone = device_zone();
                let mut abbreviation = [0u8; 8];
                abbreviation[..zone.abbreviation.len()].copy_from_slice(zone.abbreviation.as_bytes());
                Ok(to_calendar(posix_argument(request)?, zone.offset, abbreviation).map_or_else(
                    || failure(RESULT_TIME_OUT_OF_RANGE),
                    |data| ServiceReply { data, ..ServiceReply::default() },
                ))
            }
            // SetDeviceLocationName needs time:s-level permission and
            // persistent settings, which are not implemented.
            command => Err(ServiceError::UnsupportedCommand { service: "ITimeZoneService", command }),
        }
    }
}
