//! JT/T 808 wire codec: frames, BCD helpers, location/registration parsing and
//! downlink body builders. This module has no database, network or session
//! state so it can be reused by the gateway and simulator.

#![allow(dead_code)]

use chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
use thiserror::Error;

pub const MSG_TERMINAL_REGISTER: u16 = 0x0100;
pub const MSG_TERMINAL_AUTH: u16 = 0x0102;
pub const MSG_QUERY_PARAMETERS_RESPONSE: u16 = 0x0104;
pub const MSG_QUERY_ATTRIBUTES_RESPONSE: u16 = 0x0107;
pub const MSG_UPGRADE_RESULT: u16 = 0x0108;
pub const MSG_VEHICLE_CONTROL_ACK: u16 = 0x0500;
pub const MSG_SATELLITE_STATUS: u16 = 0x0205;
pub const MSG_HEARTBEAT: u16 = 0x0002;
pub const MSG_TERMINAL_RESPONSE: u16 = 0x0001;
pub const MSG_LOCATION_REPORT: u16 = 0x0200;
pub const MSG_LOCATION_BATCH: u16 = 0x0704;
pub const MSG_LOCATION_QUERY_RESPONSE: u16 = 0x0201;
pub const MSG_SET_PARAMETERS: u16 = 0x8103;
pub const MSG_QUERY_PARAMETERS: u16 = 0x8104;
pub const MSG_TERMINAL_CONTROL: u16 = 0x8105;
pub const MSG_QUERY_SPECIFIC_PARAMETERS: u16 = 0x8106;
pub const MSG_QUERY_TERMINAL_ATTRIBUTES: u16 = 0x8107;
pub const MSG_LOCATION_QUERY: u16 = 0x8201;
pub const MSG_TEMP_TRACKING: u16 = 0x8202;
pub const MSG_ALARM_ACK: u16 = 0x8203;
pub const MSG_LINK_TEST: u16 = 0x8204;
pub const MSG_TEXT_COMMAND: u16 = 0x8300;
pub const MSG_PHONE_CALL: u16 = 0x8400;
pub const MSG_INFO_SERVICE: u16 = 0x8304;
pub const MSG_CIRCLE_AREA: u16 = 0x8600;
pub const MSG_DELETE_CIRCLE_AREA: u16 = 0x8601;
pub const MSG_RECT_AREA: u16 = 0x8602;
pub const MSG_DELETE_RECT_AREA: u16 = 0x8603;
pub const MSG_POLYGON_AREA: u16 = 0x8604;
pub const MSG_DELETE_POLYGON_AREA: u16 = 0x8605;
pub const MSG_ROUTE: u16 = 0x8606;
pub const MSG_DELETE_ROUTE: u16 = 0x8607;
pub const MSG_QUERY_AREA_ROUTE: u16 = 0x8608;
pub const MSG_QUERY_AREA_ROUTE_ACK: u16 = 0x0608;
pub const MSG_TACHOGRAPH_COMMAND: u16 = 0x8700;
pub const MSG_TACHOGRAPH_UPLOAD: u16 = 0x0700;
pub const MSG_MEDIA_UPLOAD_ACK: u16 = 0x8800;
pub const MSG_CAMERA_CAPTURE: u16 = 0x8801;
pub const MSG_CAMERA_CAPTURE_ACK: u16 = 0x0805;
pub const MSG_MEDIA_SEARCH: u16 = 0x8802;
pub const MSG_MEDIA_SEARCH_ACK: u16 = 0x0802;
pub const MSG_MEDIA_UPLOAD_COMMAND: u16 = 0x8803;
pub const MSG_AUDIO_RECORD: u16 = 0x8804;
pub const MSG_SINGLE_MEDIA_UPLOAD: u16 = 0x8805;
pub const MSG_TRANSPARENT_DOWN: u16 = 0x8900;
pub const MSG_TRANSPARENT_UP: u16 = 0x0900;
pub const MSG_ELECTRONIC_WAYBILL: u16 = 0x0701;
pub const MSG_CAN_UPLOAD: u16 = 0x0705;
pub const MSG_MEDIA_EVENT: u16 = 0x0800;
pub const MSG_MEDIA_DATA: u16 = 0x0801;
pub const MSG_COMPRESSED_UPLOAD: u16 = 0x0901;
pub const MSG_PLATFORM_RSA: u16 = 0x8A00;
pub const MSG_TERMINAL_RSA: u16 = 0x0A00;
pub const MSG_DRIVER_ID_REQUEST: u16 = 0x8702;
pub const MSG_DRIVER_ID_UPLOAD: u16 = 0x0702;
pub const MSG_EVENT_SET: u16 = 0x8301;
pub const MSG_QUESTION: u16 = 0x8302;
pub const MSG_INFO_MENU: u16 = 0x8303;
pub const MSG_VEHICLE_CONTROL: u16 = 0x8500;
pub const MSG_GENERAL_RESPONSE: u16 = 0x8001;
pub const MSG_REGISTER_RESPONSE: u16 = 0x8100;

/// Bound a single 0x0704 batch to keep a malformed frame from unbounded work.
pub const MAX_BATCH_LOCATIONS: usize = 200;

/// Bound the raw 0x0704 body before parsing nested location records.
pub const MAX_BATCH_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("frame is too short")]
    TooShort,
    #[error("missing frame delimiter")]
    MissingDelimiter,
    #[error("invalid escape sequence")]
    InvalidEscape,
    #[error("checksum mismatch")]
    ChecksumMismatch,
    #[error("unsupported sub-package message")]
    SubPackageUnsupported,
    #[error("encrypted bodies are not supported")]
    EncryptedUnsupported,
    #[error("body length mismatch")]
    BodyLengthMismatch,
    #[error("invalid BCD terminal id")]
    InvalidTerminalId,
    #[error("invalid location body")]
    InvalidLocation,
    #[error("invalid location time")]
    InvalidTime,
    #[error("invalid registration body")]
    InvalidRegistration,
    #[error("invalid parameter body")]
    InvalidParameters,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub msg_id: u16,
    pub terminal_id: String,
    pub serial: u16,
    pub body: Vec<u8>,
    /// JT/T 808 sub-package metadata: (total_packets, packet_index).
    pub subpackage: Option<(u16, u16)>,
}

/// 0x0200 alarm bits (JT/T 808-2019 table 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlarmFlags(pub u32);

impl AlarmFlags {
    pub fn active(self) -> Vec<&'static str> {
        let names = [
            "emergency",
            "overspeed",
            "fatigue",
            "dangerous_driving",
            "gnss_fault",
            "gnss_antenna_disconnected",
            "gnss_antenna_short",
            "main_power_undervoltage",
            "main_power_lost",
            "lcd_fault",
            "camera_fault",
            "ic_card_fault",
            "overspeed_warning",
            "fatigue_warning",
            "illegal_driving",
            "tire_pressure_warning",
            "right_blind_spot",
            "daily_driving_overtime",
            "parking_overtime",
            "area_in_out",
            "route_in_out",
            "segment_time",
            "route_deviation",
            "vss_fault",
            "fuel_abnormal",
            "theft",
            "illegal_ignition",
            "illegal_movement",
            "collision_rollover",
            "rollover_warning",
        ];
        names
            .iter()
            .enumerate()
            .filter(|(bit, _)| self.0 & (1 << bit) != 0)
            .map(|(_, name)| *name)
            .collect()
    }
}

/// 0x0200 status bits (JT/T 808-2019 table 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusFlags(pub u32);

impl StatusFlags {
    pub fn acc_on(self) -> bool {
        self.0 & 0x0000_0001 != 0
    }
    pub fn positioned(self) -> bool {
        self.0 & 0x0000_0002 != 0
    }
    pub fn south(self) -> bool {
        self.0 & 0x0000_0004 != 0
    }
    pub fn west(self) -> bool {
        self.0 & 0x0000_0008 != 0
    }
    pub fn stopped(self) -> bool {
        self.0 & 0x0000_0010 != 0
    }
    pub fn encrypted(self) -> bool {
        self.0 & 0x0000_0020 != 0
    }
    pub fn doors_open(self) -> u8 {
        ((self.0 >> 22) & 0x1F) as u8
    }
    pub fn gnss_mask(self) -> u8 {
        ((self.0 >> 18) & 0x0F) as u8
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocationData {
    pub lat: f64,
    pub lon: f64,
    pub speed: f64,
    pub heading: f64,
    pub ts: String,
    pub ignition: bool,
    pub alarm: u32,
    pub mileage_km: Option<f64>,
    pub satellites: Option<u8>,
    /// JSON map of additional items (id -> uppercase hex value).
    pub extra_json: Option<String>,
}

impl LocationData {
    fn extra_u64(&self, key: &str) -> Option<u64> {
        let raw = self.extra_json.as_deref()?;
        let value: serde_json::Value = serde_json::from_str(raw).ok()?;
        value.get(key)?.as_u64()
    }

    /// TLV 0x25 extended vehicle signal status.
    pub fn extended_signal(&self) -> Option<u32> {
        self.extra_u64("extended_signal").map(|value| value as u32)
    }

    /// TLV 0x30 network signal strength.
    pub fn network_signal(&self) -> Option<u8> {
        self.extra_u64("signal").map(|value| value as u8)
    }

    /// TLV 0x31 satellite count.
    pub fn satellite_count(&self) -> Option<u8> {
        self.extra_u64("satellites").map(|value| value as u8)
    }
}

#[derive(Debug, Clone)]
pub struct RegistrationData {
    pub layout: String,
    pub province: u16,
    pub city: u16,
    pub manufacturer: String,
    pub model: String,
    pub terminal_id: String,
    pub plate_color: u8,
    pub plate_no: String,
}

#[derive(Debug, Clone)]
pub struct TerminalAttributes {
    pub layout: String,
    pub terminal_type: u16,
    pub manufacturer: String,
    pub model: String,
    pub terminal_id: String,
    pub iccid: String,
    pub hardware_version: String,
    pub firmware_version: String,
    pub gnss: Option<u8>,
    pub raw_hex: String,
}

fn xor_checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, byte| acc ^ byte)
}

fn unescape(data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let mut out = Vec::with_capacity(data.len());
    let mut index = 0;
    while index < data.len() {
        if data[index] == 0x7d {
            let next = *data.get(index + 1).ok_or(ProtocolError::InvalidEscape)?;
            match next {
                0x01 => out.push(0x7d),
                0x02 => out.push(0x7e),
                _ => return Err(ProtocolError::InvalidEscape),
            }
            index += 2;
        } else {
            out.push(data[index]);
            index += 1;
        }
    }
    Ok(out)
}

fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    for byte in data {
        match byte {
            0x7e => {
                out.push(0x7d);
                out.push(0x02);
            }
            0x7d => {
                out.push(0x7d);
                out.push(0x01);
            }
            _ => out.push(*byte),
        }
    }
    out
}

fn decode_bcd_phone(bytes: &[u8]) -> Result<String, ProtocolError> {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let high = byte >> 4;
        let low = byte & 0x0f;
        if high > 9 || low > 9 {
            return Err(ProtocolError::InvalidTerminalId);
        }
        out.push(char::from(b'0' + high));
        out.push(char::from(b'0' + low));
    }
    Ok(out)
}

fn encode_bcd_phone(phone: &str) -> Result<[u8; 6], ProtocolError> {
    if phone.len() != 12 || !phone.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ProtocolError::InvalidTerminalId);
    }
    let mut out = [0u8; 6];
    for (index, byte) in out.iter_mut().enumerate() {
        let high = phone.as_bytes()[index * 2] - b'0';
        let low = phone.as_bytes()[index * 2 + 1] - b'0';
        *byte = (high << 4) | low;
    }
    Ok(out)
}

pub fn parse_frame(raw: &[u8]) -> Result<Frame, ProtocolError> {
    let data = if raw.first() == Some(&0x7e) || raw.last() == Some(&0x7e) {
        let start = raw
            .iter()
            .position(|byte| *byte == 0x7e)
            .ok_or(ProtocolError::MissingDelimiter)?;
        let end = raw
            .iter()
            .rposition(|byte| *byte == 0x7e)
            .ok_or(ProtocolError::MissingDelimiter)?;
        if end <= start + 1 {
            return Err(ProtocolError::TooShort);
        }
        &raw[start + 1..end]
    } else {
        raw
    };

    let mut decoded = unescape(data)?;
    if decoded.len() < 13 {
        return Err(ProtocolError::TooShort);
    }
    let checksum_index = decoded.len() - 1;
    let expected = xor_checksum(&decoded[..checksum_index]);
    if expected != decoded[checksum_index] {
        return Err(ProtocolError::ChecksumMismatch);
    }

    let msg_id = u16::from_be_bytes([decoded[0], decoded[1]]);
    let body_props = u16::from_be_bytes([decoded[2], decoded[3]]);
    let body_len = (body_props & 0x03ff) as usize;
    if decoded.len() != 12 + body_len + 1 {
        return Err(ProtocolError::BodyLengthMismatch);
    }
    let terminal_id = decode_bcd_phone(&decoded[4..10])?;
    let serial = u16::from_be_bytes([decoded[10], decoded[11]]);
    let subpackage = if body_props & 0x2000 != 0 {
        if body_len < 4 {
            return Err(ProtocolError::BodyLengthMismatch);
        }
        let total = u16::from_be_bytes([decoded[12], decoded[13]]);
        let index = u16::from_be_bytes([decoded[14], decoded[15]]);
        if total == 0 || index == 0 || index > total {
            return Err(ProtocolError::SubPackageUnsupported);
        }
        Some((total, index))
    } else {
        None
    };
    if (body_props >> 10) & 0x07 != 0 {
        return Err(ProtocolError::EncryptedUnsupported);
    }
    // Move the body out of the decoded frame instead of allocating and copying
    // a second `Vec` for it: dropping the 12-byte header and the trailing
    // checksum is a short in-place memmove, no allocation.
    decoded.drain(..12);
    decoded.truncate(body_len);
    if subpackage.is_some() {
        decoded.drain(..4);
    }
    Ok(Frame {
        msg_id,
        terminal_id,
        serial,
        body: decoded,
        subpackage,
    })
}

pub fn build_frame(
    msg_id: u16,
    terminal_id: &str,
    serial: u16,
    body: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    if body.len() > 1023 {
        return Err(ProtocolError::BodyLengthMismatch);
    }
    let phone = encode_bcd_phone(terminal_id)?;
    let mut plain = Vec::with_capacity(12 + body.len() + 1);
    plain.extend_from_slice(&msg_id.to_be_bytes());
    plain.extend_from_slice(&(body.len() as u16).to_be_bytes());
    plain.extend_from_slice(&phone);
    plain.extend_from_slice(&serial.to_be_bytes());
    plain.extend_from_slice(body);
    let checksum = xor_checksum(&plain);
    plain.push(checksum);

    let mut framed = Vec::with_capacity(plain.len() * 2 + 2);
    framed.push(0x7e);
    framed.extend_from_slice(&escape(&plain));
    framed.push(0x7e);
    Ok(framed)
}

fn parse_bcd_time(bytes: &[u8]) -> Result<String, ProtocolError> {
    if bytes.len() < 6 {
        return Err(ProtocolError::InvalidTime);
    }
    let digits: Vec<u8> = bytes[..6]
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .collect();
    if digits.iter().any(|digit| *digit > 9) {
        return Err(ProtocolError::InvalidTime);
    }
    let number = |start: usize, len: usize| -> u32 {
        digits[start..start + len]
            .iter()
            .fold(0u32, |acc, digit| acc * 10 + *digit as u32)
    };
    let year = 2000 + number(0, 2) as i32;
    let month = number(2, 2);
    let day = number(4, 2);
    let hour = number(6, 2);
    let minute = number(8, 2);
    let second = number(10, 2);
    let date = NaiveDate::from_ymd_opt(year, month, day).ok_or(ProtocolError::InvalidTime)?;
    let naive = date
        .and_hms_opt(hour, minute, second)
        .ok_or(ProtocolError::InvalidTime)?;
    let offset = FixedOffset::east_opt(8 * 3600).expect("valid offset");
    let local = offset
        .from_local_datetime(&naive)
        .single()
        .ok_or(ProtocolError::InvalidTime)?;
    Ok(local
        .with_timezone(&Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

pub fn parse_location(body: &[u8]) -> Result<LocationData, ProtocolError> {
    if body.len() < 28 {
        return Err(ProtocolError::InvalidLocation);
    }
    let alarm = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
    let status = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let lat_raw = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
    let lon_raw = u32::from_be_bytes([body[12], body[13], body[14], body[15]]);
    let speed_raw = u16::from_be_bytes([body[18], body[19]]);
    let heading_raw = u16::from_be_bytes([body[20], body[21]]);
    let ts = parse_bcd_time(&body[22..28])?;

    let mut lat = lat_raw as f64 / 1_000_000.0;
    let mut lon = lon_raw as f64 / 1_000_000.0;
    if status & 0x0000_0004 != 0 {
        lat = -lat;
    }
    if status & 0x0000_0008 != 0 {
        lon = -lon;
    }
    let ignition = status & 1 != 0;
    let speed = speed_raw as f64 / 10.0;
    let heading = heading_raw as f64;
    if !(-90.0..=90.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lon)
        || speed > 250.0
        || heading > 359.0
    {
        return Err(ProtocolError::InvalidLocation);
    }

    let mut mileage_km = None;
    let mut satellites = None;
    let mut fuel_l = None;
    let mut driving_record_km = None;
    let mut alarm_id = None;
    let mut extended_signal = None;
    let mut signal = None;
    let mut overspeed = None;
    let mut area_event = None;
    let mut route_time = None;
    let mut extras = serde_json::Map::new();
    let mut index = 28;
    while index + 2 <= body.len() {
        let id = body[index];
        let len = body[index + 1] as usize;
        index += 2;
        if index + len > body.len() {
            return Err(ProtocolError::InvalidLocation);
        }
        let value = &body[index..index + len];
        match id {
            0x01 if len >= 4 => {
                let raw = u32::from_be_bytes([value[0], value[1], value[2], value[3]]);
                mileage_km = Some(raw as f64 / 10.0);
            }
            0x02 if len >= 2 => {
                fuel_l = Some(u16::from_be_bytes([value[0], value[1]]) as f64 / 10.0);
            }
            0x04 if len >= 4 => {
                alarm_id = Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
            }
            0x05 if !value.is_empty() => {
                extras.insert(
                    "tire_pressure_pa".to_string(),
                    serde_json::json!(value.iter().map(|v| *v as u32).collect::<Vec<_>>()),
                );
            }
            0x06 if len >= 2 => {
                let raw = u16::from_be_bytes([value[0], value[1]]) as i16;
                extras.insert("cabin_temp_c".to_string(), serde_json::json!(raw));
            }
            0x2A if len >= 2 => {
                extras.insert(
                    "io_status".to_string(),
                    serde_json::json!(u16::from_be_bytes([value[0], value[1]])),
                );
            }
            0x2B if len >= 4 => {
                extras.insert(
                    "analog_ad0".to_string(),
                    serde_json::json!(u16::from_be_bytes([value[0], value[1]])),
                );
                extras.insert(
                    "analog_ad1".to_string(),
                    serde_json::json!(u16::from_be_bytes([value[2], value[3]])),
                );
            }
            0x11 if len >= 5 => {
                overspeed = Some((
                    value[0],
                    u32::from_be_bytes([value[1], value[2], value[3], value[4]]),
                ));
            }
            0x12 if len >= 6 => {
                area_event = Some((
                    value[0],
                    u32::from_be_bytes([value[1], value[2], value[3], value[4]]),
                    value[5],
                ));
            }
            0x13 if len >= 8 => {
                route_time = Some((
                    value[0],
                    u32::from_be_bytes([value[1], value[2], value[3], value[4]]),
                    u16::from_be_bytes([value[5], value[6]]),
                    value[7],
                ));
            }
            0x03 if len >= 2 => {
                driving_record_km = Some(u16::from_be_bytes([value[0], value[1]]) as f64 / 10.0);
            }
            0x25 if len >= 4 => {
                extended_signal =
                    Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
            }
            0x30 if len >= 1 => {
                signal = Some(value[0]);
            }
            0x31 if len >= 1 => {
                satellites = Some(value[0]);
            }
            _ => {}
        }
        extras.insert(format!("{id:02X}"), serde_json::Value::String(hex(value)));
        index += len;
    }
    if let Some(value) = mileage_km {
        extras.insert("mileage_km".to_string(), serde_json::json!(value));
    }
    if let Some(value) = fuel_l {
        extras.insert("fuel_l".to_string(), serde_json::json!(value));
    }
    if let Some(value) = driving_record_km {
        extras.insert("driving_record_km".to_string(), serde_json::json!(value));
    }
    if let Some(value) = alarm_id {
        extras.insert("alarm_id".to_string(), serde_json::json!(value));
    }
    if let Some((area_type, area_id)) = overspeed {
        extras.insert(
            "overspeed_area_type".to_string(),
            serde_json::json!(area_type),
        );
        extras.insert("overspeed_area_id".to_string(), serde_json::json!(area_id));
    }
    if let Some((area_type, area_id, direction)) = area_event {
        extras.insert("area_event_type".to_string(), serde_json::json!(area_type));
        extras.insert("area_id".to_string(), serde_json::json!(area_id));
        extras.insert("area_direction".to_string(), serde_json::json!(direction));
    }
    if let Some((route_type, route_id, seconds, result)) = route_time {
        extras.insert("route_time_type".to_string(), serde_json::json!(route_type));
        extras.insert("route_id".to_string(), serde_json::json!(route_id));
        extras.insert("route_time_seconds".to_string(), serde_json::json!(seconds));
        extras.insert("route_time_result".to_string(), serde_json::json!(result));
    }
    if let Some(value) = extended_signal {
        extras.insert("extended_signal".to_string(), serde_json::json!(value));
    }
    if let Some(value) = signal {
        extras.insert("signal".to_string(), serde_json::json!(value));
    }
    if let Some(value) = satellites {
        extras.insert("satellites".to_string(), serde_json::json!(value));
    }
    let extra_json = if extras.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(extras).to_string())
    };

    Ok(LocationData {
        lat,
        lon,
        speed,
        heading,
        ts,
        ignition,
        alarm,
        mileage_km,
        satellites,
        extra_json,
    })
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

pub fn parse_registration(body: &[u8]) -> Result<RegistrationData, ProtocolError> {
    // Both layouts appear in field deployments. 37-byte is the legacy layout
    // used by earlier implementations; 76-byte is the 2019 candidate layout
    // (province/city + manufacturer 11 + model 30 + terminal id 30 + color + plate).
    let clean = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .trim_matches('\0')
            .trim()
            .to_string()
    };
    if body.len() >= 76 {
        return Ok(RegistrationData {
            layout: "2019-table8".to_string(),
            province: u16::from_be_bytes([body[0], body[1]]),
            city: u16::from_be_bytes([body[2], body[3]]),
            manufacturer: clean(&body[4..15]),
            model: clean(&body[15..45]),
            terminal_id: clean(&body[45..75]),
            plate_color: body[75],
            plate_no: if body.len() > 76 {
                let (text, _, _) = encoding_rs::GBK.decode(&body[76..]);
                text.trim_matches('\0').trim().to_string()
            } else {
                String::new()
            },
        });
    }
    if body.len() >= 37 {
        return Ok(RegistrationData {
            layout: "legacy-37".to_string(),
            province: u16::from_be_bytes([body[0], body[1]]),
            city: u16::from_be_bytes([body[2], body[3]]),
            manufacturer: clean(&body[4..9]),
            model: clean(&body[9..29]),
            terminal_id: hex(&body[29..36]),
            plate_color: body[36],
            plate_no: String::new(),
        });
    }
    Err(ProtocolError::InvalidRegistration)
}

/// 0x0107 terminal attributes, 2019 candidate layout. Unknown tails are kept
/// in `raw_hex`; callers must not treat this as conformance-tested.
pub fn parse_terminal_attributes(body: &[u8]) -> Result<TerminalAttributes, ProtocolError> {
    if body.len() < 79 {
        return Err(ProtocolError::InvalidParameters);
    }
    let clean = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .trim_matches('\0')
            .trim()
            .to_string()
    };
    let mut index = 77;
    let hardware_len = body[index] as usize;
    index += 1;
    if index + hardware_len + 1 > body.len() {
        return Err(ProtocolError::InvalidParameters);
    }
    let hardware_version = clean(&body[index..index + hardware_len]);
    index += hardware_len;
    let firmware_len = body[index] as usize;
    index += 1;
    if index + firmware_len + 2 > body.len() {
        return Err(ProtocolError::InvalidParameters);
    }
    let firmware_version = clean(&body[index..index + firmware_len]);
    index += firmware_len;
    let gnss = body.get(index).copied();
    Ok(TerminalAttributes {
        layout: "2019-table20".to_string(),
        terminal_type: u16::from_be_bytes([body[0], body[1]]),
        manufacturer: clean(&body[2..7]),
        model: clean(&body[7..37]),
        terminal_id: clean(&body[37..67]),
        iccid: bcd_digits(&body[67..77]),
        hardware_version,
        firmware_version,
        gnss,
        raw_hex: hex(body),
    })
}

fn bcd_digits(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(b'0' + (byte >> 4)));
        out.push(char::from(b'0' + (byte & 0x0F)));
    }
    out
}

/// 0x0704 batch location upload: count(2), type(1), then repeated
/// length(2)-prefixed 0x0200 bodies.
pub fn parse_location_batch(body: &[u8]) -> Result<Vec<LocationData>, ProtocolError> {
    Ok(parse_location_batch_typed(body)?.items)
}

/// 0x0704 batch body: count + data type (0 normal, 1 blind) + length-prefixed locations.
#[derive(Debug, Clone, PartialEq)]
pub struct LocationBatch {
    pub data_type: u8,
    pub items: Vec<LocationData>,
}

pub fn parse_location_batch_typed(body: &[u8]) -> Result<LocationBatch, ProtocolError> {
    if body.len() < 3 || body.len() > MAX_BATCH_BODY_BYTES {
        return Err(ProtocolError::InvalidLocation);
    }
    let count = u16::from_be_bytes([body[0], body[1]]) as usize;
    if count == 0 || count > MAX_BATCH_LOCATIONS {
        return Err(ProtocolError::InvalidLocation);
    }
    let data_type = body[2];
    let mut index = 3;
    let mut out = Vec::with_capacity(count.min(100));
    for _ in 0..count {
        if index + 2 > body.len() {
            return Err(ProtocolError::InvalidLocation);
        }
        let len = u16::from_be_bytes([body[index], body[index + 1]]) as usize;
        index += 2;
        if index + len > body.len() {
            return Err(ProtocolError::InvalidLocation);
        }
        out.push(parse_location(&body[index..index + len])?);
        index += len;
    }
    if index != body.len() {
        return Err(ProtocolError::InvalidLocation);
    }
    Ok(LocationBatch {
        data_type,
        items: out,
    })
}

/// Build a 0x0201 location query response body from current vehicle state.
pub fn encode_location_response(
    lat: f64,
    lon: f64,
    speed: f64,
    heading: f64,
    ignition: bool,
    ts: &str,
) -> Vec<u8> {
    let offset = FixedOffset::east_opt(8 * 3600).expect("valid offset");
    let local = chrono::DateTime::parse_from_rfc3339(ts)
        .map(|value| value.with_timezone(&offset))
        .unwrap_or_else(|_| Utc::now().with_timezone(&offset));
    let digits = local.format("%y%m%d%H%M%S").to_string();
    let mut bcd = [0u8; 6];
    for (index, chunk) in digits.as_bytes().chunks(2).enumerate().take(6) {
        if chunk.len() == 2 {
            bcd[index] = ((chunk[0] - b'0') << 4) | (chunk[1] - b'0');
        }
    }
    let mut status: u32 = 0x0000_0002; // positioned
    if ignition {
        status |= 0x0000_0001;
    }
    if lat < 0.0 {
        status |= 0x0000_0004;
    }
    if lon < 0.0 {
        status |= 0x0000_0008;
    }
    let mut body = Vec::with_capacity(28);
    body.extend_from_slice(&0u32.to_be_bytes()); // alarm
    body.extend_from_slice(&status.to_be_bytes());
    body.extend_from_slice(&((lat.abs() * 1_000_000.0) as u32).to_be_bytes());
    body.extend_from_slice(&((lon.abs() * 1_000_000.0) as u32).to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes()); // altitude (unknown)
    body.extend_from_slice(&((speed.max(0.0) * 10.0) as u16).to_be_bytes());
    body.extend_from_slice(&((heading.rem_euclid(360.0)) as u16).to_be_bytes());
    body.extend_from_slice(&bcd);
    body
}

#[cfg(test)]
mod flag_tests {
    use super::*;

    #[test]
    fn alarm_and_status_bits_follow_2019_tables() {
        let alarm = AlarmFlags((1 << 0) | (1 << 1) | (1 << 14) | (1 << 29));
        assert_eq!(
            alarm.active(),
            vec![
                "emergency",
                "overspeed",
                "illegal_driving",
                "rollover_warning"
            ]
        );
        let status =
            StatusFlags(0x0000_0001 | 0x0000_0002 | 0x0000_0020 | (0b0101 << 18) | (0b00011 << 22));
        assert!(status.acc_on() && status.positioned() && status.encrypted());
        assert_eq!(status.gnss_mask(), 0b0101);
        assert_eq!(status.doors_open(), 0b00011);
        assert!(!status.south() && !status.west() && !status.stopped());
    }

    #[test]
    fn location_parses_tire_pressure_and_cabin_temperature() {
        let mut body = vec![0u8; 28];
        body[0..4].copy_from_slice(&0u32.to_be_bytes());
        body[4..8].copy_from_slice(&2u32.to_be_bytes());
        body[22..28].copy_from_slice(&[0x26, 0x11, 0x23, 0x12, 0x00, 0x00]);
        body.extend_from_slice(&[0x05, 0x03, 210, 215, 220]);
        body.extend_from_slice(&[0x06, 0x02, 0xFF, 0xF6]);
        let location = parse_location(&body).unwrap();
        let extras = location.extra_json.expect("extras");
        let json: serde_json::Value = serde_json::from_str(&extras).unwrap();
        assert_eq!(json["tire_pressure_pa"], serde_json::json!([210, 215, 220]));
        assert_eq!(json["cabin_temp_c"], serde_json::json!(-10));
    }

    #[test]
    fn location_parses_io_and_analog() {
        let mut body = vec![0u8; 28];
        body[4..8].copy_from_slice(&2u32.to_be_bytes());
        body[22..28].copy_from_slice(&[0x26, 0x11, 0x23, 0x12, 0x00, 0x00]);
        body.extend_from_slice(&[0x2A, 0x02, 0x80, 0x01]);
        body.extend_from_slice(&[0x2B, 0x04, 0x00, 0x64, 0x00, 0xC8]);
        let location = parse_location(&body).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(location.extra_json.as_deref().unwrap()).unwrap();
        assert_eq!(json["io_status"], serde_json::json!(32769));
        assert_eq!(json["analog_ad0"], serde_json::json!(100));
        assert_eq!(json["analog_ad1"], serde_json::json!(200));
    }

    #[test]
    fn location_batch_preserves_type_and_rejects_trailing() {
        let mut item = vec![0u8; 28];
        item[4..8].copy_from_slice(&2u32.to_be_bytes());
        item[22..28].copy_from_slice(&[0x26, 0x11, 0x23, 0x12, 0x00, 0x00]);
        let mut body = Vec::new();
        body.extend_from_slice(&1u16.to_be_bytes());
        body.push(0x01);
        body.extend_from_slice(&(item.len() as u16).to_be_bytes());
        body.extend_from_slice(&item);
        let batch = parse_location_batch_typed(&body).unwrap();
        assert_eq!(batch.data_type, 1);
        assert_eq!(batch.items.len(), 1);
        let mut trailing = body.clone();
        trailing.push(0xFF);
        assert!(parse_location_batch_typed(&trailing).is_err());
    }

    #[test]
    fn location_exposes_extended_signal_and_network_tlvs() {
        let mut body = vec![0u8; 28];
        body[4..8].copy_from_slice(&2u32.to_be_bytes());
        body[22..28].copy_from_slice(&[0x26, 0x11, 0x23, 0x12, 0x00, 0x00]);
        body.push(0x25);
        body.push(4);
        body.extend_from_slice(&0x11223344u32.to_be_bytes());
        body.push(0x30);
        body.push(1);
        body.push(23);
        let location = parse_location(&body).unwrap();
        assert_eq!(location.extended_signal(), Some(0x11223344));
        assert_eq!(location.network_signal(), Some(23));
    }
}
