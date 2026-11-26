//! JT/T 808 downlink parameter/control bodies (0x8103/0x8105/0x8106/0x8202/0x8500).

use super::codec::ProtocolError;

mod media;
pub use media::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalParameter {
    pub id: u32,
    pub value: Vec<u8>,
}

/// One satellite entry in 0x0205 table 89.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SatelliteState {
    pub id: u8,
    pub elevation: u8,
    pub azimuth: u16,
    pub cnr: u8,
}

/// 0x0205 satellite status parameters (JT/T 808-2019 amendment 1, tables 88/89).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SatelliteStatus {
    pub beidou: Vec<SatelliteState>,
    pub gps: Vec<SatelliteState>,
    pub glonass: Vec<SatelliteState>,
    pub galileo: Vec<SatelliteState>,
    pub location: [u8; 28],
}

impl SatelliteState {
    fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.id);
        out.push(self.elevation);
        out.extend_from_slice(&self.azimuth.to_be_bytes());
        out.push(self.cnr);
    }
}

pub fn encode_satellite_status(status: &SatelliteStatus) -> Result<Vec<u8>, ProtocolError> {
    for group in [
        &status.beidou,
        &status.gps,
        &status.glonass,
        &status.galileo,
    ] {
        if group.len() > 100 {
            return Err(ProtocolError::InvalidParameters);
        }
    }
    let mut body = Vec::with_capacity(
        4 + 5
            * (status.beidou.len()
                + status.gps.len()
                + status.glonass.len()
                + status.galileo.len())
            + 28,
    );
    for group in [
        &status.beidou,
        &status.gps,
        &status.glonass,
        &status.galileo,
    ] {
        body.push(group.len() as u8);
        for satellite in group {
            satellite.encode_into(&mut body);
        }
    }
    body.extend_from_slice(&status.location);
    Ok(body)
}

pub fn decode_satellite_status(body: &[u8]) -> Result<SatelliteStatus, ProtocolError> {
    let mut index = 0usize;
    let mut groups = Vec::with_capacity(4);
    for _ in 0..4 {
        if index >= body.len() {
            return Err(ProtocolError::InvalidParameters);
        }
        let count = body[index] as usize;
        index += 1;
        if count > 100 || index + count * 5 > body.len() {
            return Err(ProtocolError::InvalidParameters);
        }
        let mut group = Vec::with_capacity(count);
        for _ in 0..count {
            let id = body[index];
            let elevation = body[index + 1];
            let azimuth = u16::from_be_bytes([body[index + 2], body[index + 3]]);
            let cnr = body[index + 4];
            if !(1..=200).contains(&id) || elevation > 90 || azimuth > 359 || cnr > 99 {
                return Err(ProtocolError::InvalidParameters);
            }
            group.push(SatelliteState {
                id,
                elevation,
                azimuth,
                cnr,
            });
            index += 5;
        }
        groups.push(group);
    }
    if body.len() != index + 28 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(SatelliteStatus {
        beidou: groups.remove(0),
        gps: groups.remove(0),
        glonass: groups.remove(0),
        galileo: groups.remove(0),
        location: body[index..].try_into().unwrap(),
    })
}

/// 0x0500 vehicle control ACK: serial + 0x0200 location body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VehicleControlAck {
    pub serial: u16,
    pub location: Vec<u8>,
}

pub fn encode_vehicle_control_ack(ack: &VehicleControlAck) -> Result<Vec<u8>, ProtocolError> {
    if ack.location.len() < 28 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(2 + ack.location.len());
    body.extend_from_slice(&ack.serial.to_be_bytes());
    body.extend_from_slice(&ack.location);
    Ok(body)
}

pub fn decode_vehicle_control_ack(body: &[u8]) -> Result<VehicleControlAck, ProtocolError> {
    if body.len() < 30 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(VehicleControlAck {
        serial: u16::from_be_bytes([body[0], body[1]]),
        location: body[2..].to_vec(),
    })
}

/// 0x0104 query terminal parameters response: serial + parameter list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalParameterResponse {
    pub serial: u16,
    pub parameters: Vec<TerminalParameter>,
}

pub fn encode_terminal_parameter_response(
    response: &TerminalParameterResponse,
) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Vec::new();
    body.extend_from_slice(&response.serial.to_be_bytes());
    body.extend_from_slice(&encode_terminal_parameters(&response.parameters)?);
    Ok(body)
}

pub fn decode_terminal_parameter_response(
    body: &[u8],
) -> Result<TerminalParameterResponse, ProtocolError> {
    if body.len() < 3 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(TerminalParameterResponse {
        serial: u16::from_be_bytes([body[0], body[1]]),
        parameters: decode_terminal_parameters(&body[2..])?,
    })
}

/// 0x0107 query terminal attributes response (JT/T 808-2019 table 20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalAttributeReport {
    pub terminal_type: u16,
    pub manufacturer: [u8; 5],
    pub model: [u8; 30],
    pub terminal_id: [u8; 30],
    pub iccid: [u8; 10],
    pub hardware_version: String,
    pub firmware_version: String,
    pub gnss_attr: u8,
    pub comm_attr: u8,
}

pub fn encode_terminal_attribute_report(
    attrs: &TerminalAttributeReport,
) -> Result<Vec<u8>, ProtocolError> {
    let (hw, _, hw_bad) = encoding_rs::GBK.encode(&attrs.hardware_version);
    let (fw, _, fw_bad) = encoding_rs::GBK.encode(&attrs.firmware_version);
    if hw_bad || fw_bad || hw.len() > u8::MAX as usize || fw.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(81 + hw.len() + fw.len());
    body.extend_from_slice(&attrs.terminal_type.to_be_bytes());
    body.extend_from_slice(&attrs.manufacturer);
    body.extend_from_slice(&attrs.model);
    body.extend_from_slice(&attrs.terminal_id);
    body.extend_from_slice(&attrs.iccid);
    body.push(hw.len() as u8);
    body.extend_from_slice(&hw);
    body.push(fw.len() as u8);
    body.extend_from_slice(&fw);
    body.push(attrs.gnss_attr);
    body.push(attrs.comm_attr);
    Ok(body)
}

pub fn decode_terminal_attribute_report(
    body: &[u8],
) -> Result<TerminalAttributeReport, ProtocolError> {
    if body.len() < 79 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut offset = 2 + 5 + 30 + 30 + 10;
    if body.len() < offset + 1 {
        return Err(ProtocolError::InvalidParameters);
    }
    let hw_len = body[offset] as usize;
    offset += 1;
    if body.len() < offset + hw_len + 1 {
        return Err(ProtocolError::InvalidParameters);
    }
    let (hw, _, hw_bad) = encoding_rs::GBK.decode(&body[offset..offset + hw_len]);
    offset += hw_len;
    let fw_len = body[offset] as usize;
    offset += 1;
    if body.len() < offset + fw_len + 2 || hw_bad {
        return Err(ProtocolError::InvalidParameters);
    }
    let (fw, _, fw_bad) = encoding_rs::GBK.decode(&body[offset..offset + fw_len]);
    offset += fw_len;
    if fw_bad || body.len() != offset + 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(TerminalAttributeReport {
        terminal_type: u16::from_be_bytes([body[0], body[1]]),
        manufacturer: body[2..7].try_into().unwrap(),
        model: body[7..37].try_into().unwrap(),
        terminal_id: body[37..67].try_into().unwrap(),
        iccid: body[67..77].try_into().unwrap(),
        hardware_version: hw.into_owned(),
        firmware_version: fw.into_owned(),
        gnss_attr: body[offset],
        comm_attr: body[offset + 1],
    })
}

/// 0x0108 terminal upgrade result: upgrade type + result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpgradeResult {
    pub upgrade_type: u8,
    pub result: u8,
}

pub fn encode_upgrade_result(result: &UpgradeResult) -> Result<Vec<u8>, ProtocolError> {
    if result.result > 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(vec![result.upgrade_type, result.result])
}

pub fn decode_upgrade_result(body: &[u8]) -> Result<UpgradeResult, ProtocolError> {
    if body.len() != 2 || body[1] > 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(UpgradeResult {
        upgrade_type: body[0],
        result: body[1],
    })
}

/// 0x8103 terminal parameter body.
pub fn encode_terminal_parameters(
    parameters: &[TerminalParameter],
) -> Result<Vec<u8>, ProtocolError> {
    if parameters.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::new();
    body.push(parameters.len() as u8);
    for parameter in parameters {
        if parameter.value.len() > u8::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.extend_from_slice(&parameter.id.to_be_bytes());
        body.push(parameter.value.len() as u8);
        body.extend_from_slice(&parameter.value);
    }
    Ok(body)
}

#[allow(dead_code)]
pub fn decode_terminal_parameters(body: &[u8]) -> Result<Vec<TerminalParameter>, ProtocolError> {
    let Some((&count, mut rest)) = body.split_first() else {
        return Err(ProtocolError::InvalidParameters);
    };
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if rest.len() < 5 {
            return Err(ProtocolError::InvalidParameters);
        }
        let id = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let len = rest[4] as usize;
        if rest.len() < 5 + len {
            return Err(ProtocolError::InvalidParameters);
        }
        out.push(TerminalParameter {
            id,
            value: rest[5..5 + len].to_vec(),
        });
        rest = &rest[5 + len..];
    }
    Ok(out)
}

/// 0x8105 terminal control body: one control flag followed by vendor/standard
/// parameter bytes.
pub fn encode_terminal_control(flag: u8, parameter: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(parameter.len() + 1);
    body.push(flag);
    body.extend_from_slice(parameter);
    body
}

/// 0x8106 query-specific-parameter body.
pub fn encode_specific_parameter_query(ids: &[u32]) -> Result<Vec<u8>, ProtocolError> {
    if ids.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(ids.len() * 4 + 1);
    body.push(ids.len() as u8);
    for id in ids {
        body.extend_from_slice(&id.to_be_bytes());
    }
    Ok(body)
}

/// 0x8202 temporary tracking body: interval seconds + expiry seconds.
#[allow(dead_code)]
pub fn encode_temp_tracking(interval_seconds: u16, expiry_seconds: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(6);
    body.extend_from_slice(&interval_seconds.to_be_bytes());
    body.extend_from_slice(&expiry_seconds.to_be_bytes());
    body
}

/// 0x8500 vehicle control body. The exact control-flag bit assignment is
/// vendor/standard-profile specific; callers pass the flag explicitly.
pub fn encode_vehicle_control(flag: u8, parameter: &[u8]) -> Vec<u8> {
    encode_terminal_control(flag, parameter)
}

/// Vendor-neutral action profile used by the HTTP API. The flag values are the
/// profile currently used by the demo T-Box; deployments with a different
/// T-Box profile should send the raw `flag` instead.
pub fn vehicle_control_action(action: &str) -> Option<(u8, Vec<u8>)> {
    let (flag, parameter) = match action {
        "lock" => (0x01, vec![0x01]),
        "unlock" => (0x01, vec![0x00]),
        "start" => (0x02, vec![0x01]),
        "stop" => (0x02, vec![0x00]),
        "find" => (0x04, vec![0x01]),
        "trunk_open" => (0x08, vec![0x01]),
        "trunk_close" => (0x08, vec![0x00]),
        "ac_on" => (0x10, vec![0x01]),
        "ac_off" => (0x10, vec![0x00]),
        _ => return None,
    };
    Some((flag, parameter))
}

/// 0x8104 query all terminal parameters: empty body.
pub fn encode_query_all_parameters() -> Vec<u8> {
    Vec::new()
}

/// 0x8107 query terminal attributes: empty body.
pub fn encode_query_terminal_attributes() -> Vec<u8> {
    Vec::new()
}

/// 0x8300 text command body: flag(u8) + GBK text. The 2019 text length is
/// bounded to 1024 bytes here to keep a malformed frame from allocating
/// unbounded memory.
pub fn encode_text_command(flag: u8, text: &str) -> Result<Vec<u8>, ProtocolError> {
    let (encoded, _, _) = encoding_rs::GBK.encode(text);
    if encoded.len() > 1024 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(encoded.len() + 1);
    body.push(flag);
    body.extend_from_slice(&encoded);
    Ok(body)
}

/// Candidate 0x8301 event item. Field widths follow public 2011/2019 tables
/// but must be checked against the licensed 2019 full text before conformance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventItem {
    pub id: u8,
    pub event_type: u8,
    pub content: String,
}

pub fn encode_event_set(items: &[EventItem]) -> Result<Vec<u8>, ProtocolError> {
    if items.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![items.len() as u8];
    for item in items {
        let (encoded, _, had_errors) = encoding_rs::GBK.encode(&item.content);
        if had_errors || encoded.len() > u8::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.push(item.id);
        body.push(item.event_type);
        body.push(encoded.len() as u8);
        body.extend_from_slice(&encoded);
    }
    Ok(body)
}

pub fn decode_event_set(body: &[u8]) -> Result<Vec<EventItem>, ProtocolError> {
    let Some((&count, mut rest)) = body.split_first() else {
        return Err(ProtocolError::InvalidParameters);
    };
    let mut items = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if rest.len() < 3 {
            return Err(ProtocolError::InvalidParameters);
        }
        let id = rest[0];
        let event_type = rest[1];
        let len = rest[2] as usize;
        if rest.len() < 3 + len {
            return Err(ProtocolError::InvalidParameters);
        }
        let (content, _, had_errors) = encoding_rs::GBK.decode(&rest[3..3 + len]);
        if had_errors {
            return Err(ProtocolError::InvalidParameters);
        }
        items.push(EventItem {
            id,
            event_type,
            content: content.into_owned(),
        });
        rest = &rest[3 + len..];
    }
    Ok(items)
}

/// Candidate 0x8302 question body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub flag: u8,
    pub text: String,
    pub answers: Vec<(u8, String)>,
}

pub fn encode_question(question: &Question) -> Result<Vec<u8>, ProtocolError> {
    let (text, _, had_errors) = encoding_rs::GBK.encode(&question.text);
    if had_errors || text.is_empty() || text.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    if question.answers.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![question.flag, text.len() as u8];
    body.extend_from_slice(&text);
    body.push(question.answers.len() as u8);
    for (id, answer) in &question.answers {
        let (encoded, _, had_errors) = encoding_rs::GBK.encode(answer);
        if had_errors || encoded.len() > u8::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.push(*id);
        body.push(encoded.len() as u8);
        body.extend_from_slice(&encoded);
    }
    Ok(body)
}

pub fn decode_question(body: &[u8]) -> Result<Question, ProtocolError> {
    if body.len() < 3 {
        return Err(ProtocolError::InvalidParameters);
    }
    let flag = body[0];
    let text_len = body[1] as usize;
    if body.len() < 2 + text_len + 1 {
        return Err(ProtocolError::InvalidParameters);
    }
    let (text, _, had_errors) = encoding_rs::GBK.decode(&body[2..2 + text_len]);
    if had_errors {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut rest = &body[2 + text_len..];
    let answer_count = rest[0] as usize;
    rest = &rest[1..];
    let mut answers = Vec::with_capacity(answer_count);
    for _ in 0..answer_count {
        if rest.len() < 2 {
            return Err(ProtocolError::InvalidParameters);
        }
        let id = rest[0];
        let len = rest[1] as usize;
        if rest.len() < 2 + len {
            return Err(ProtocolError::InvalidParameters);
        }
        let (answer, _, had_errors) = encoding_rs::GBK.decode(&rest[2..2 + len]);
        if had_errors {
            return Err(ProtocolError::InvalidParameters);
        }
        answers.push((id, answer.into_owned()));
        rest = &rest[2 + len..];
    }
    Ok(Question {
        flag,
        text: text.into_owned(),
        answers,
    })
}

/// Candidate 0x8303 information-on-demand menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoMenuItem {
    pub info_type: u8,
    pub name: String,
}

pub fn encode_info_menu(set_type: u8, items: &[InfoMenuItem]) -> Result<Vec<u8>, ProtocolError> {
    if items.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![set_type, items.len() as u8];
    for item in items {
        let (encoded, _, had_errors) = encoding_rs::GBK.encode(&item.name);
        if had_errors || encoded.len() > u8::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.push(item.info_type);
        body.push(encoded.len() as u8);
        body.extend_from_slice(&encoded);
    }
    Ok(body)
}

pub fn decode_info_menu(body: &[u8]) -> Result<(u8, Vec<InfoMenuItem>), ProtocolError> {
    if body.len() < 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let set_type = body[0];
    let count = body[1] as usize;
    let mut rest = &body[2..];
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        if rest.len() < 2 {
            return Err(ProtocolError::InvalidParameters);
        }
        let info_type = rest[0];
        let len = rest[1] as usize;
        if rest.len() < 2 + len {
            return Err(ProtocolError::InvalidParameters);
        }
        let (name, _, had_errors) = encoding_rs::GBK.decode(&rest[2..2 + len]);
        if had_errors {
            return Err(ProtocolError::InvalidParameters);
        }
        items.push(InfoMenuItem {
            info_type,
            name: name.into_owned(),
        });
        rest = &rest[2 + len..];
    }
    Ok((set_type, items))
}

/// 0x8600 circular area item (JT/T 808-2019 table 46).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircleArea {
    pub id: u32,
    pub attr: u16,
    pub lat: i32,
    pub lon: i32,
    pub radius: u32,
    pub start: [u8; 6],
    pub end: [u8; 6],
    pub max_speed: u16,
    pub overspeed: u16,
    pub night_speed: u16,
    pub name: String,
}

pub fn encode_circle_areas(set_attr: u8, areas: &[CircleArea]) -> Result<Vec<u8>, ProtocolError> {
    if set_attr > 2 || areas.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![set_attr, areas.len() as u8];
    for area in areas {
        body.extend_from_slice(&area.id.to_be_bytes());
        body.extend_from_slice(&area.attr.to_be_bytes());
        body.extend_from_slice(&area.lat.to_be_bytes());
        body.extend_from_slice(&area.lon.to_be_bytes());
        body.extend_from_slice(&area.radius.to_be_bytes());
        if area.attr & 0x01 != 0 {
            body.extend_from_slice(&area.start);
            body.extend_from_slice(&area.end);
        }
        if area.attr & 0x02 != 0 {
            body.extend_from_slice(&area.max_speed.to_be_bytes());
            body.extend_from_slice(&area.overspeed.to_be_bytes());
            body.extend_from_slice(&area.night_speed.to_be_bytes());
        }
        let (name, _, had_errors) = encoding_rs::GBK.encode(&area.name);
        if had_errors || name.len() > u8::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.push(name.len() as u8);
        body.extend_from_slice(&name);
    }
    Ok(body)
}

pub fn decode_circle_areas(body: &[u8]) -> Result<(u8, Vec<CircleArea>), ProtocolError> {
    if body.len() < 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let set_attr = body[0];
    let count = body[1] as usize;
    let mut rest = &body[2..];
    let mut areas = Vec::with_capacity(count);
    for _ in 0..count {
        if rest.len() < 19 {
            return Err(ProtocolError::InvalidParameters);
        }
        let id = u32::from_be_bytes(rest[0..4].try_into().unwrap());
        let attr = u16::from_be_bytes(rest[4..6].try_into().unwrap());
        let lat = i32::from_be_bytes(rest[6..10].try_into().unwrap());
        let lon = i32::from_be_bytes(rest[10..14].try_into().unwrap());
        let radius = u32::from_be_bytes(rest[14..18].try_into().unwrap());
        let mut offset = 18;
        let mut start = [0u8; 6];
        let mut end = [0u8; 6];
        if attr & 0x01 != 0 {
            if rest.len() < offset + 12 {
                return Err(ProtocolError::InvalidParameters);
            }
            start.copy_from_slice(&rest[offset..offset + 6]);
            end.copy_from_slice(&rest[offset + 6..offset + 12]);
            offset += 12;
        }
        let mut max_speed = 0;
        let mut overspeed = 0;
        let mut night_speed = 0;
        if attr & 0x02 != 0 {
            if rest.len() < offset + 6 {
                return Err(ProtocolError::InvalidParameters);
            }
            max_speed = u16::from_be_bytes(rest[offset..offset + 2].try_into().unwrap());
            overspeed = u16::from_be_bytes(rest[offset + 2..offset + 4].try_into().unwrap());
            night_speed = u16::from_be_bytes(rest[offset + 4..offset + 6].try_into().unwrap());
            offset += 6;
        }
        if rest.len() < offset + 1 {
            return Err(ProtocolError::InvalidParameters);
        }
        let name_len = rest[offset] as usize;
        offset += 1;
        if rest.len() < offset + name_len {
            return Err(ProtocolError::InvalidParameters);
        }
        let (name, _, had_errors) = encoding_rs::GBK.decode(&rest[offset..offset + name_len]);
        if had_errors {
            return Err(ProtocolError::InvalidParameters);
        }
        areas.push(CircleArea {
            id,
            attr,
            lat,
            lon,
            radius,
            start,
            end,
            max_speed,
            overspeed,
            night_speed,
            name: name.into_owned(),
        });
        rest = &rest[offset + name_len..];
    }
    Ok((set_attr, areas))
}

/// 0x0702 driver identity upload (JT/T 808-2019 table 66).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverIdentity {
    pub status: u8,
    pub time: [u8; 6],
    pub card_result: u8,
    pub name: String,
    pub qualification: String,
    pub agency: String,
    pub valid_date: [u8; 4],
    pub id_card: String,
    /// Amendment 1: face-to-ID photo match 0..=100.
    pub face_match: Option<u8>,
    /// Amendment 1: identity-card UID, fixed 20 bytes on the wire.
    pub uid: String,
}

pub fn encode_driver_identity(identity: &DriverIdentity) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Vec::new();
    body.push(identity.status);
    body.extend_from_slice(&identity.time);
    body.push(identity.card_result);
    if identity.status != 0x01 || identity.card_result != 0x00 {
        return Ok(body);
    }
    let (name, _, e1) = encoding_rs::GBK.encode(&identity.name);
    let (agency, _, e2) = encoding_rs::GBK.encode(&identity.agency);
    if e1 || e2 || name.len() > u8::MAX as usize || agency.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let cert = fixed_bytes(&identity.qualification, 20)?;
    let id_card = fixed_bytes(&identity.id_card, 20)?;
    body.push(name.len() as u8);
    body.extend_from_slice(&name);
    body.extend_from_slice(&cert);
    body.push(agency.len() as u8);
    body.extend_from_slice(&agency);
    body.extend_from_slice(&identity.valid_date);
    body.extend_from_slice(&id_card);
    if let Some(score) = identity.face_match {
        if score > 100 {
            return Err(ProtocolError::InvalidParameters);
        }
        body.push(score);
        if !identity.uid.is_empty() {
            body.extend_from_slice(&fixed_bytes(&identity.uid, 20)?);
        }
    } else if !identity.uid.is_empty() {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(body)
}

fn fixed_bytes(value: &str, size: usize) -> Result<Vec<u8>, ProtocolError> {
    let (bytes, _, had_errors) = encoding_rs::GBK.encode(value);
    if had_errors || bytes.len() > size {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut out = vec![0u8; size];
    out[..bytes.len()].copy_from_slice(&bytes);
    Ok(out)
}

pub fn decode_driver_identity(body: &[u8]) -> Result<DriverIdentity, ProtocolError> {
    if body.len() < 8 {
        return Err(ProtocolError::InvalidParameters);
    }
    let status = body[0];
    let time = body[1..7].try_into().unwrap();
    let card_result = body[7];
    if status != 0x01 || card_result != 0x00 {
        if body.len() != 8 {
            return Err(ProtocolError::InvalidParameters);
        }
        return Ok(DriverIdentity {
            status,
            time,
            card_result,
            name: String::new(),
            qualification: String::new(),
            agency: String::new(),
            valid_date: [0; 4],
            id_card: String::new(),
            face_match: None,
            uid: String::new(),
        });
    }
    if body.len() < 8 + 1 + 20 + 1 + 4 + 20 {
        return Err(ProtocolError::InvalidParameters);
    }
    let name_len = body[8] as usize;
    if body.len() < 9 + name_len + 20 + 1 {
        return Err(ProtocolError::InvalidParameters);
    }
    let (name, _, e1) = encoding_rs::GBK.decode(&body[9..9 + name_len]);
    let cert_start = 9 + name_len;
    let (qualification, _, e2) =
        encoding_rs::GBK.decode(trim_nul(&body[cert_start..cert_start + 20]));
    let agency_len_at = cert_start + 20;
    let agency_len = body[agency_len_at] as usize;
    if body.len() < agency_len_at + 1 + agency_len + 4 + 20 || e1 || e2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let agency_start = agency_len_at + 1;
    let (agency, _, e3) = encoding_rs::GBK.decode(&body[agency_start..agency_start + agency_len]);
    let date_at = agency_start + agency_len;
    let valid_date = body[date_at..date_at + 4].try_into().unwrap();
    let id_start = date_at + 4;
    let id_end = id_start + 20;
    let (id_card, _, e4) = encoding_rs::GBK.decode(trim_nul(&body[id_start..id_end]));
    if e3 || e4 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut face_match = None;
    let mut uid = String::new();
    if body.len() > id_end {
        let score = body[id_end];
        if score > 100 {
            return Err(ProtocolError::InvalidParameters);
        }
        face_match = Some(score);
        if body.len() > id_end + 1 {
            if body.len() != id_end + 1 + 20 {
                return Err(ProtocolError::InvalidParameters);
            }
            let (value, _, bad) = encoding_rs::GBK.decode(trim_nul(&body[id_end + 1..id_end + 21]));
            if bad {
                return Err(ProtocolError::InvalidParameters);
            }
            uid = value.into_owned();
        }
    }
    Ok(DriverIdentity {
        status,
        time,
        card_result,
        name: name.into_owned(),
        qualification: qualification.into_owned(),
        agency: agency.into_owned(),
        valid_date,
        id_card: id_card.into_owned(),
        face_match,
        uid,
    })
}

fn trim_nul(data: &[u8]) -> &[u8] {
    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    &data[..end]
}

/// 0x0901 compressed upload (JT/T 808-2019 table 85): length DWORD + GZIP body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedUpload {
    pub data: Vec<u8>,
}

pub fn encode_compressed_upload(upload: &CompressedUpload) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Vec::with_capacity(4 + upload.data.len());
    body.extend_from_slice(&(upload.data.len() as u32).to_be_bytes());
    body.extend_from_slice(&upload.data);
    Ok(body)
}

pub fn decode_compressed_upload(body: &[u8]) -> Result<CompressedUpload, ProtocolError> {
    if body.len() < 4 {
        return Err(ProtocolError::InvalidParameters);
    }
    let len = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
    if body.len() != 4 + len || len < 2 || body[4] != 0x1f || body[5] != 0x8b {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(CompressedUpload {
        data: body[4..].to_vec(),
    })
}

pub fn gzip_compress(data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(data)
        .map_err(|_| ProtocolError::InvalidParameters)?;
    encoder
        .finish()
        .map_err(|_| ProtocolError::InvalidParameters)
}

pub fn gzip_decompress(data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|_| ProtocolError::InvalidParameters)?;
    Ok(out)
}

/// 0x8A00/0x0A00 RSA public key: length DWORD + 128-byte modulus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsaPublicKeyMessage {
    pub modulus: [u8; 128],
}

pub fn encode_rsa_public_key(key: &RsaPublicKeyMessage) -> Vec<u8> {
    let mut body = Vec::with_capacity(132);
    body.extend_from_slice(&128u32.to_be_bytes());
    body.extend_from_slice(&key.modulus);
    body
}

pub fn decode_rsa_public_key(body: &[u8]) -> Result<RsaPublicKeyMessage, ProtocolError> {
    if body.len() != 132 || u32::from_be_bytes(body[0..4].try_into().unwrap()) != 128 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(RsaPublicKeyMessage {
        modulus: body[4..132].try_into().unwrap(),
    })
}

/// 0x0705 CAN bus upload (JT/T 808-2019 tables 69/70).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanItem {
    pub id: u32,
    pub data: [u8; 8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanUpload {
    pub time: [u8; 5],
    pub items: Vec<CanItem>,
}

pub fn encode_can_upload(upload: &CanUpload) -> Result<Vec<u8>, ProtocolError> {
    if upload.items.is_empty() || upload.items.len() > u16::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(7 + upload.items.len() * 12);
    body.extend_from_slice(&(upload.items.len() as u16).to_be_bytes());
    body.extend_from_slice(&upload.time);
    for item in &upload.items {
        body.extend_from_slice(&item.id.to_be_bytes());
        body.extend_from_slice(&item.data);
    }
    Ok(body)
}

pub fn decode_can_upload(body: &[u8]) -> Result<CanUpload, ProtocolError> {
    if body.len() < 7 {
        return Err(ProtocolError::InvalidParameters);
    }
    let count = u16::from_be_bytes([body[0], body[1]]) as usize;
    if count == 0 || body.len() != 7 + count * 12 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut items = Vec::with_capacity(count);
    let mut offset = 7;
    for _ in 0..count {
        items.push(CanItem {
            id: u32::from_be_bytes(body[offset..offset + 4].try_into().unwrap()),
            data: body[offset + 4..offset + 12].try_into().unwrap(),
        });
        offset += 12;
    }
    Ok(CanUpload {
        time: body[2..7].try_into().unwrap(),
        items,
    })
}

/// 0x0802 item with optional 0x0200 TLVs. Only a single item is accepted,
/// because the table has no per-item length prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationTlv {
    pub id: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSearchItemExtended {
    pub media_id: u32,
    pub media_type: u8,
    pub channel: u8,
    pub event: u8,
    pub location: [u8; 28],
    pub tlvs: Vec<LocationTlv>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSearchAckExtended {
    pub serial: u16,
    pub total: u16,
    pub item: MediaSearchItemExtended,
}

pub fn encode_media_search_ack_extended(
    ack: &MediaSearchAckExtended,
) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Vec::new();
    body.extend_from_slice(&ack.serial.to_be_bytes());
    body.extend_from_slice(&ack.total.to_be_bytes());
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&ack.item.media_id.to_be_bytes());
    body.extend_from_slice(&[ack.item.media_type, ack.item.channel, ack.item.event]);
    body.extend_from_slice(&ack.item.location);
    for tlv in &ack.item.tlvs {
        if tlv.data.len() > u8::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.push(tlv.id);
        body.push(tlv.data.len() as u8);
        body.extend_from_slice(&tlv.data);
    }
    Ok(body)
}

pub fn decode_media_search_ack_extended(
    body: &[u8],
) -> Result<MediaSearchAckExtended, ProtocolError> {
    if body.len() < 41 {
        return Err(ProtocolError::InvalidParameters);
    }
    let serial = u16::from_be_bytes([body[0], body[1]]);
    let total = u16::from_be_bytes([body[2], body[3]]);
    if u16::from_be_bytes([body[4], body[5]]) != 1 {
        return Err(ProtocolError::InvalidParameters);
    }
    let media_id = u32::from_be_bytes(body[6..10].try_into().unwrap());
    let media_type = body[10];
    let channel = body[11];
    let event = body[12];
    let location = body[13..41].try_into().unwrap();
    let mut tlvs = Vec::new();
    let mut offset = 41;
    while offset < body.len() {
        if body.len() < offset + 2 {
            return Err(ProtocolError::InvalidParameters);
        }
        let id = body[offset];
        let len = body[offset + 1] as usize;
        offset += 2;
        if body.len() < offset + len {
            return Err(ProtocolError::InvalidParameters);
        }
        tlvs.push(LocationTlv {
            id,
            data: body[offset..offset + len].to_vec(),
        });
        offset += len;
    }
    Ok(MediaSearchAckExtended {
        serial,
        total,
        item: MediaSearchItemExtended {
            media_id,
            media_type,
            channel,
            event,
            location,
            tlvs,
        },
    })
}

/// 0x0800 multimedia event (JT/T 808-2019 table 71).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEvent {
    pub media_id: u32,
    pub media_type: u8,
    pub format: u8,
    pub event: u8,
    pub channel: u8,
}

pub fn encode_media_event(event: &MediaEvent) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&event.media_id.to_be_bytes());
    body.extend_from_slice(&[event.media_type, event.format, event.event, event.channel]);
    body
}

pub fn decode_media_event(body: &[u8]) -> Result<MediaEvent, ProtocolError> {
    if body.len() != 8 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(MediaEvent {
        media_id: u32::from_be_bytes(body[0..4].try_into().unwrap()),
        media_type: body[4],
        format: body[5],
        event: body[6],
        channel: body[7],
    })
}

/// 0x0801 multimedia data upload (JT/T 808-2019 table 72).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaData {
    pub media_id: u32,
    pub media_type: u8,
    pub format: u8,
    pub event: u8,
    pub channel: u8,
    pub location: [u8; 28],
    pub data: Vec<u8>,
}

pub fn encode_media_data(media: &MediaData) -> Vec<u8> {
    let mut body = Vec::with_capacity(36 + media.data.len());
    body.extend_from_slice(&media.media_id.to_be_bytes());
    body.extend_from_slice(&[media.media_type, media.format, media.event, media.channel]);
    body.extend_from_slice(&media.location);
    body.extend_from_slice(&media.data);
    body
}

pub fn decode_media_data(body: &[u8]) -> Result<MediaData, ProtocolError> {
    if body.len() < 36 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(MediaData {
        media_id: u32::from_be_bytes(body[0..4].try_into().unwrap()),
        media_type: body[4],
        format: body[5],
        event: body[6],
        channel: body[7],
        location: body[8..36].try_into().unwrap(),
        data: body[36..].to_vec(),
    })
}

/// 0x0701 electronic waybill upload (JT/T 808-2019 table 65).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElectronicWaybill {
    pub data: Vec<u8>,
}

pub fn encode_electronic_waybill(waybill: &ElectronicWaybill) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Vec::with_capacity(4 + waybill.data.len());
    body.extend_from_slice(&(waybill.data.len() as u32).to_be_bytes());
    body.extend_from_slice(&waybill.data);
    Ok(body)
}

pub fn decode_electronic_waybill(body: &[u8]) -> Result<ElectronicWaybill, ProtocolError> {
    if body.len() < 4 {
        return Err(ProtocolError::InvalidParameters);
    }
    let len = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
    if body.len() != 4 + len {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(ElectronicWaybill {
        data: body[4..].to_vec(),
    })
}

/// 0x8702 driver identity request has an empty body.
pub fn encode_driver_id_request() -> Vec<u8> {
    Vec::new()
}

/// 0x8700 tachograph data acquisition command (JT/T 808-2019 table 62).
/// The data block is an opaque GB/T 19056 packet and is passed through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TachographCommand {
    pub command: u8,
    pub data: Vec<u8>,
}

pub fn encode_tachograph_command(command: &TachographCommand) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + command.data.len());
    body.push(command.command);
    body.extend_from_slice(&command.data);
    body
}

pub fn decode_tachograph_command(body: &[u8]) -> Result<TachographCommand, ProtocolError> {
    let Some((&command, data)) = body.split_first() else {
        return Err(ProtocolError::InvalidParameters);
    };
    Ok(TachographCommand {
        command,
        data: data.to_vec(),
    })
}

/// 0x0700 tachograph data upload (JT/T 808-2019 table 63).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TachographUpload {
    pub serial: u16,
    pub command: u8,
    pub data: Vec<u8>,
}

pub fn encode_tachograph_upload(upload: &TachographUpload) -> Vec<u8> {
    let mut body = Vec::with_capacity(3 + upload.data.len());
    body.extend_from_slice(&upload.serial.to_be_bytes());
    body.push(upload.command);
    body.extend_from_slice(&upload.data);
    body
}

impl TachographUpload {
    /// Decode the data block as a 19056-2021 record block (55 76 header).
    pub fn record_block(
        &self,
    ) -> Result<crate::standards::gb19056::Gb19056RecordBlock, ProtocolError> {
        crate::standards::gb19056::Gb19056RecordBlock::decode(&self.data)
            .map_err(|_| ProtocolError::InvalidParameters)
    }
}

pub fn decode_tachograph_upload(body: &[u8]) -> Result<TachographUpload, ProtocolError> {
    if body.len() < 3 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(TachographUpload {
        serial: u16::from_be_bytes([body[0], body[1]]),
        command: body[2],
        data: body[3..].to_vec(),
    })
}

/// 0x0608 area/route query ACK. Each DATA block is self-delimiting by its
/// own count/optional fields, so blocks can be parsed sequentially.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaQueryBlock {
    Circle {
        set_attr: u8,
        areas: Vec<CircleArea>,
    },
    Rect {
        set_attr: u8,
        areas: Vec<RectArea>,
    },
    Polygon {
        set_attr: u8,
        areas: Vec<PolygonArea>,
    },
    Route(Route),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaQueryAck {
    pub query_type: u8,
    pub total: u32,
    pub blocks: Vec<AreaQueryBlock>,
}

pub fn encode_area_query_ack(ack: &AreaQueryAck) -> Result<Vec<u8>, ProtocolError> {
    if ack.blocks.len() as u32 != ack.total {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![ack.query_type];
    body.extend_from_slice(&ack.total.to_be_bytes());
    for block in &ack.blocks {
        match (&ack.query_type, block) {
            (1, AreaQueryBlock::Circle { set_attr, areas }) => {
                body.extend_from_slice(&encode_circle_areas(*set_attr, areas)?)
            }
            (2, AreaQueryBlock::Rect { set_attr, areas }) => {
                body.extend_from_slice(&encode_rect_areas(*set_attr, areas)?)
            }
            (3, AreaQueryBlock::Polygon { set_attr, areas }) => {
                body.extend_from_slice(&encode_polygon_areas(*set_attr, areas)?)
            }
            (4, AreaQueryBlock::Route(route)) => body.extend_from_slice(&encode_route(route)?),
            _ => return Err(ProtocolError::InvalidParameters),
        }
    }
    Ok(body)
}

pub fn decode_area_query_ack(body: &[u8]) -> Result<AreaQueryAck, ProtocolError> {
    if body.len() < 6 {
        return Err(ProtocolError::InvalidParameters);
    }
    let query_type = body[0];
    let total = u32::from_be_bytes(body[1..5].try_into().unwrap());
    let mut offset = 5usize;
    let mut blocks = Vec::with_capacity(total as usize);
    for _ in 0..total {
        let len = area_block_len(&body[offset..], query_type)?;
        if len == 0 || offset + len > body.len() {
            return Err(ProtocolError::InvalidParameters);
        }
        let payload = &body[offset..offset + len];
        blocks.push(match query_type {
            1 => {
                let (set_attr, areas) = decode_circle_areas(payload)?;
                AreaQueryBlock::Circle { set_attr, areas }
            }
            2 => {
                let (set_attr, areas) = decode_rect_areas(payload)?;
                AreaQueryBlock::Rect { set_attr, areas }
            }
            3 => {
                let (set_attr, areas) = decode_polygon_areas(payload)?;
                AreaQueryBlock::Polygon { set_attr, areas }
            }
            4 => AreaQueryBlock::Route(decode_route(payload)?),
            _ => return Err(ProtocolError::InvalidParameters),
        });
        offset += len;
    }
    if offset != body.len() {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(AreaQueryAck {
        query_type,
        total,
        blocks,
    })
}

fn area_block_len(body: &[u8], query_type: u8) -> Result<usize, ProtocolError> {
    match query_type {
        1 => {
            if body.len() < 2 {
                return Err(ProtocolError::InvalidParameters);
            }
            let count = body[1] as usize;
            let mut offset = 2;
            for _ in 0..count {
                if body.len() < offset + 19 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let attr = u16::from_be_bytes([body[offset + 4], body[offset + 5]]);
                let mut item = 18;
                if attr & 0x01 != 0 {
                    item += 12;
                }
                if attr & 0x02 != 0 {
                    item += 6;
                }
                if body.len() < offset + item + 1 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let name = body[offset + item] as usize;
                item += 1 + name;
                offset += item;
            }
            Ok(offset)
        }
        2 => {
            if body.len() < 2 {
                return Err(ProtocolError::InvalidParameters);
            }
            let count = body[1] as usize;
            let mut offset = 2;
            for _ in 0..count {
                if body.len() < offset + 24 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let attr = u16::from_be_bytes([body[offset + 4], body[offset + 5]]);
                let mut item = 22;
                if attr & 0x01 != 0 {
                    item += 12;
                }
                if attr & 0x02 != 0 {
                    item += 5;
                }
                if body.len() < offset + item + 2 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let name =
                    u16::from_be_bytes([body[offset + item], body[offset + item + 1]]) as usize;
                item += 2 + name;
                offset += item;
            }
            Ok(offset)
        }
        3 => {
            if body.len() < 2 {
                return Err(ProtocolError::InvalidParameters);
            }
            let count = body[1] as usize;
            let mut offset = 2;
            for _ in 0..count {
                if body.len() < offset + 8 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let attr = u16::from_be_bytes([body[offset + 4], body[offset + 5]]);
                let mut item = 6;
                if attr & 0x01 != 0 {
                    item += 12;
                }
                if attr & 0x02 != 0 {
                    item += 3;
                }
                if body.len() < offset + item + 2 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let vertices =
                    u16::from_be_bytes([body[offset + item], body[offset + item + 1]]) as usize;
                item += 2 + vertices * 8;
                if attr & 0x02 != 0 {
                    item += 2;
                }
                if body.len() < offset + item + 2 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let name =
                    u16::from_be_bytes([body[offset + item], body[offset + item + 1]]) as usize;
                item += 2 + name;
                offset += item;
            }
            Ok(offset)
        }
        4 => {
            if body.len() < 8 {
                return Err(ProtocolError::InvalidParameters);
            }
            let attr = u16::from_be_bytes([body[4], body[5]]);
            let mut offset = 6;
            if attr & 0x01 != 0 {
                offset += 12;
            }
            if body.len() < offset + 2 {
                return Err(ProtocolError::InvalidParameters);
            }
            let points = u16::from_be_bytes([body[offset], body[offset + 1]]) as usize;
            offset += 2;
            for _ in 0..points {
                if body.len() < offset + 18 {
                    return Err(ProtocolError::InvalidParameters);
                }
                let pattr = body[offset + 17];
                let mut item = 18;
                if pattr & 0x01 != 0 {
                    item += 4;
                }
                if pattr & 0x02 != 0 {
                    item += 6;
                }
                offset += item;
            }
            if body.len() < offset + 2 {
                return Err(ProtocolError::InvalidParameters);
            }
            let name = u16::from_be_bytes([body[offset], body[offset + 1]]) as usize;
            Ok(offset + 2 + name)
        }
        _ => Err(ProtocolError::InvalidParameters),
    }
}

/// 0x8608 query area/route body (JT/T 808-2019 table 60).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaQuery {
    pub query_type: u8,
    pub ids: Vec<u32>,
}

pub fn encode_area_query(query: &AreaQuery) -> Result<Vec<u8>, ProtocolError> {
    if !(1..=4).contains(&query.query_type) || query.ids.len() > 125 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(5 + query.ids.len() * 4);
    body.push(query.query_type);
    body.extend_from_slice(&(query.ids.len() as u32).to_be_bytes());
    for id in &query.ids {
        body.extend_from_slice(&id.to_be_bytes());
    }
    Ok(body)
}

pub fn decode_area_query(body: &[u8]) -> Result<AreaQuery, ProtocolError> {
    if body.len() < 5 {
        return Err(ProtocolError::InvalidParameters);
    }
    let query_type = body[0];
    if !(1..=4).contains(&query_type) {
        return Err(ProtocolError::InvalidParameters);
    }
    let count = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
    if count > 125 || body.len() != 5 + count * 4 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut ids = Vec::with_capacity(count);
    for chunk in body[5..].chunks(4) {
        ids.push(u32::from_be_bytes(chunk.try_into().unwrap()));
    }
    Ok(AreaQuery { query_type, ids })
}

/// 0x8606 route inflection point (JT/T 808-2019 table 57).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePoint {
    pub id: u32,
    pub segment_id: u32,
    pub lat: i32,
    pub lon: i32,
    pub width: u8,
    pub attr: u8,
    pub overlong: u16,
    pub under: u16,
    pub max_speed: u16,
    pub overspeed: u16,
    pub night_speed: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub id: u32,
    pub attr: u16,
    pub start: [u8; 6],
    pub end: [u8; 6],
    pub points: Vec<RoutePoint>,
    pub name: String,
}

pub fn encode_route(route: &Route) -> Result<Vec<u8>, ProtocolError> {
    if route.points.is_empty() || route.points.len() > u16::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::new();
    body.extend_from_slice(&route.id.to_be_bytes());
    body.extend_from_slice(&route.attr.to_be_bytes());
    if route.attr & 0x01 != 0 {
        body.extend_from_slice(&route.start);
        body.extend_from_slice(&route.end);
    }
    body.extend_from_slice(&(route.points.len() as u16).to_be_bytes());
    for point in &route.points {
        body.extend_from_slice(&point.id.to_be_bytes());
        body.extend_from_slice(&point.segment_id.to_be_bytes());
        body.extend_from_slice(&point.lat.to_be_bytes());
        body.extend_from_slice(&point.lon.to_be_bytes());
        body.push(point.width);
        body.push(point.attr);
        if point.attr & 0x01 != 0 {
            body.extend_from_slice(&point.overlong.to_be_bytes());
            body.extend_from_slice(&point.under.to_be_bytes());
        }
        if point.attr & 0x02 != 0 {
            body.extend_from_slice(&point.max_speed.to_be_bytes());
            body.extend_from_slice(&point.overspeed.to_be_bytes());
            body.extend_from_slice(&point.night_speed.to_be_bytes());
        }
    }
    let (name, _, had_errors) = encoding_rs::GBK.encode(&route.name);
    if had_errors || name.len() > u16::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    body.extend_from_slice(&(name.len() as u16).to_be_bytes());
    body.extend_from_slice(&name);
    Ok(body)
}

pub fn decode_route(body: &[u8]) -> Result<Route, ProtocolError> {
    if body.len() < 8 {
        return Err(ProtocolError::InvalidParameters);
    }
    let id = u32::from_be_bytes(body[0..4].try_into().unwrap());
    let attr = u16::from_be_bytes(body[4..6].try_into().unwrap());
    let mut offset = 6;
    let mut start = [0u8; 6];
    let mut end = [0u8; 6];
    if attr & 0x01 != 0 {
        if body.len() < offset + 12 {
            return Err(ProtocolError::InvalidParameters);
        }
        start.copy_from_slice(&body[offset..offset + 6]);
        end.copy_from_slice(&body[offset + 6..offset + 12]);
        offset += 12;
    }
    if body.len() < offset + 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let count = u16::from_be_bytes(body[offset..offset + 2].try_into().unwrap()) as usize;
    offset += 2;
    if count == 0 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        if body.len() < offset + 18 {
            return Err(ProtocolError::InvalidParameters);
        }
        let point_id = u32::from_be_bytes(body[offset..offset + 4].try_into().unwrap());
        let segment_id = u32::from_be_bytes(body[offset + 4..offset + 8].try_into().unwrap());
        let lat = i32::from_be_bytes(body[offset + 8..offset + 12].try_into().unwrap());
        let lon = i32::from_be_bytes(body[offset + 12..offset + 16].try_into().unwrap());
        let width = body[offset + 16];
        let point_attr = body[offset + 17];
        offset += 18;
        let mut overlong = 0;
        let mut under = 0;
        if point_attr & 0x01 != 0 {
            if body.len() < offset + 4 {
                return Err(ProtocolError::InvalidParameters);
            }
            overlong = u16::from_be_bytes(body[offset..offset + 2].try_into().unwrap());
            under = u16::from_be_bytes(body[offset + 2..offset + 4].try_into().unwrap());
            offset += 4;
        }
        let mut max_speed = 0;
        let mut overspeed = 0;
        let mut night_speed = 0;
        if point_attr & 0x02 != 0 {
            if body.len() < offset + 6 {
                return Err(ProtocolError::InvalidParameters);
            }
            max_speed = u16::from_be_bytes(body[offset..offset + 2].try_into().unwrap());
            overspeed = u16::from_be_bytes(body[offset + 2..offset + 4].try_into().unwrap());
            night_speed = u16::from_be_bytes(body[offset + 4..offset + 6].try_into().unwrap());
            offset += 6;
        }
        points.push(RoutePoint {
            id: point_id,
            segment_id,
            lat,
            lon,
            width,
            attr: point_attr,
            overlong,
            under,
            max_speed,
            overspeed,
            night_speed,
        });
    }
    if body.len() < offset + 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let name_len = u16::from_be_bytes(body[offset..offset + 2].try_into().unwrap()) as usize;
    offset += 2;
    if body.len() < offset + name_len {
        return Err(ProtocolError::InvalidParameters);
    }
    let (name, _, had_errors) = encoding_rs::GBK.decode(&body[offset..offset + name_len]);
    if had_errors {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(Route {
        id,
        attr,
        start,
        end,
        points,
        name: name.into_owned(),
    })
}

pub fn encode_delete_routes(ids: &[u32]) -> Result<Vec<u8>, ProtocolError> {
    encode_delete_circle_areas(ids)
}

/// 0x8604 polygon area item (JT/T 808-2019 tables 52/53).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolygonArea {
    pub id: u32,
    pub attr: u16,
    pub start: [u8; 6],
    pub end: [u8; 6],
    pub max_speed: u16,
    pub overspeed: u8,
    pub night_speed: u16,
    pub vertices: Vec<(i32, i32)>,
    pub name: String,
}

pub fn encode_polygon_areas(set_attr: u8, areas: &[PolygonArea]) -> Result<Vec<u8>, ProtocolError> {
    if set_attr > 2 || areas.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![set_attr, areas.len() as u8];
    for area in areas {
        if area.vertices.is_empty() || area.vertices.len() > u16::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.extend_from_slice(&area.id.to_be_bytes());
        body.extend_from_slice(&area.attr.to_be_bytes());
        if area.attr & 0x01 != 0 {
            body.extend_from_slice(&area.start);
            body.extend_from_slice(&area.end);
        }
        if area.attr & 0x02 != 0 {
            body.extend_from_slice(&area.max_speed.to_be_bytes());
            body.push(area.overspeed);
        }
        body.extend_from_slice(&(area.vertices.len() as u16).to_be_bytes());
        for (lat, lon) in &area.vertices {
            body.extend_from_slice(&lat.to_be_bytes());
            body.extend_from_slice(&lon.to_be_bytes());
        }
        if area.attr & 0x02 != 0 {
            body.extend_from_slice(&area.night_speed.to_be_bytes());
        }
        let (name, _, had_errors) = encoding_rs::GBK.encode(&area.name);
        if had_errors || name.len() > u16::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.extend_from_slice(&(name.len() as u16).to_be_bytes());
        body.extend_from_slice(&name);
    }
    Ok(body)
}

pub fn decode_polygon_areas(body: &[u8]) -> Result<(u8, Vec<PolygonArea>), ProtocolError> {
    if body.len() < 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let set_attr = body[0];
    let count = body[1] as usize;
    let mut rest = &body[2..];
    let mut areas = Vec::with_capacity(count);
    for _ in 0..count {
        if rest.len() < 6 {
            return Err(ProtocolError::InvalidParameters);
        }
        let id = u32::from_be_bytes(rest[0..4].try_into().unwrap());
        let attr = u16::from_be_bytes(rest[4..6].try_into().unwrap());
        let mut offset = 6;
        let mut start = [0u8; 6];
        let mut end = [0u8; 6];
        if attr & 0x01 != 0 {
            if rest.len() < offset + 12 {
                return Err(ProtocolError::InvalidParameters);
            }
            start.copy_from_slice(&rest[offset..offset + 6]);
            end.copy_from_slice(&rest[offset + 6..offset + 12]);
            offset += 12;
        }
        let mut max_speed = 0;
        let mut overspeed = 0;
        if attr & 0x02 != 0 {
            if rest.len() < offset + 3 {
                return Err(ProtocolError::InvalidParameters);
            }
            max_speed = u16::from_be_bytes(rest[offset..offset + 2].try_into().unwrap());
            overspeed = rest[offset + 2];
            offset += 3;
        }
        if rest.len() < offset + 2 {
            return Err(ProtocolError::InvalidParameters);
        }
        let vertex_count =
            u16::from_be_bytes(rest[offset..offset + 2].try_into().unwrap()) as usize;
        offset += 2;
        if vertex_count == 0 || rest.len() < offset + vertex_count * 8 {
            return Err(ProtocolError::InvalidParameters);
        }
        let mut vertices = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            let lat = i32::from_be_bytes(rest[offset..offset + 4].try_into().unwrap());
            let lon = i32::from_be_bytes(rest[offset + 4..offset + 8].try_into().unwrap());
            vertices.push((lat, lon));
            offset += 8;
        }
        let mut night_speed = 0;
        if attr & 0x02 != 0 {
            if rest.len() < offset + 2 {
                return Err(ProtocolError::InvalidParameters);
            }
            night_speed = u16::from_be_bytes(rest[offset..offset + 2].try_into().unwrap());
            offset += 2;
        }
        if rest.len() < offset + 2 {
            return Err(ProtocolError::InvalidParameters);
        }
        let name_len = u16::from_be_bytes(rest[offset..offset + 2].try_into().unwrap()) as usize;
        offset += 2;
        if rest.len() < offset + name_len {
            return Err(ProtocolError::InvalidParameters);
        }
        let (name, _, had_errors) = encoding_rs::GBK.decode(&rest[offset..offset + name_len]);
        if had_errors {
            return Err(ProtocolError::InvalidParameters);
        }
        areas.push(PolygonArea {
            id,
            attr,
            start,
            end,
            max_speed,
            overspeed,
            night_speed,
            vertices,
            name: name.into_owned(),
        });
        rest = &rest[offset + name_len..];
    }
    Ok((set_attr, areas))
}

pub fn encode_delete_circle_areas(ids: &[u32]) -> Result<Vec<u8>, ProtocolError> {
    if ids.len() > 125 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(1 + ids.len() * 4);
    body.push(ids.len() as u8);
    for id in ids {
        body.extend_from_slice(&id.to_be_bytes());
    }
    Ok(body)
}

pub fn encode_delete_polygon_areas(ids: &[u32]) -> Result<Vec<u8>, ProtocolError> {
    encode_delete_circle_areas(ids)
}

/// 0x8602 rectangular area item (JT/T 808-2019 table 50).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RectArea {
    pub id: u32,
    pub attr: u16,
    pub top_left_lat: i32,
    pub top_left_lon: i32,
    pub bottom_right_lat: i32,
    pub bottom_right_lon: i32,
    pub start: [u8; 6],
    pub end: [u8; 6],
    pub max_speed: u16,
    pub overspeed: u8,
    pub night_speed: u16,
    pub name: String,
}

pub fn encode_rect_areas(set_attr: u8, areas: &[RectArea]) -> Result<Vec<u8>, ProtocolError> {
    if set_attr > 2 || areas.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![set_attr, areas.len() as u8];
    for area in areas {
        body.extend_from_slice(&area.id.to_be_bytes());
        body.extend_from_slice(&area.attr.to_be_bytes());
        body.extend_from_slice(&area.top_left_lat.to_be_bytes());
        body.extend_from_slice(&area.top_left_lon.to_be_bytes());
        body.extend_from_slice(&area.bottom_right_lat.to_be_bytes());
        body.extend_from_slice(&area.bottom_right_lon.to_be_bytes());
        if area.attr & 0x01 != 0 {
            body.extend_from_slice(&area.start);
            body.extend_from_slice(&area.end);
        }
        if area.attr & 0x02 != 0 {
            body.extend_from_slice(&area.max_speed.to_be_bytes());
            body.push(area.overspeed);
            body.extend_from_slice(&area.night_speed.to_be_bytes());
        }
        let (name, _, had_errors) = encoding_rs::GBK.encode(&area.name);
        if had_errors || name.len() > u16::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.extend_from_slice(&(name.len() as u16).to_be_bytes());
        body.extend_from_slice(&name);
    }
    Ok(body)
}

pub fn decode_rect_areas(body: &[u8]) -> Result<(u8, Vec<RectArea>), ProtocolError> {
    if body.len() < 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let set_attr = body[0];
    let count = body[1] as usize;
    let mut rest = &body[2..];
    let mut areas = Vec::with_capacity(count);
    for _ in 0..count {
        if rest.len() < 24 {
            return Err(ProtocolError::InvalidParameters);
        }
        let id = u32::from_be_bytes(rest[0..4].try_into().unwrap());
        let attr = u16::from_be_bytes(rest[4..6].try_into().unwrap());
        let top_left_lat = i32::from_be_bytes(rest[6..10].try_into().unwrap());
        let top_left_lon = i32::from_be_bytes(rest[10..14].try_into().unwrap());
        let bottom_right_lat = i32::from_be_bytes(rest[14..18].try_into().unwrap());
        let bottom_right_lon = i32::from_be_bytes(rest[18..22].try_into().unwrap());
        let mut offset = 22;
        let mut start = [0u8; 6];
        let mut end = [0u8; 6];
        if attr & 0x01 != 0 {
            if rest.len() < offset + 12 {
                return Err(ProtocolError::InvalidParameters);
            }
            start.copy_from_slice(&rest[offset..offset + 6]);
            end.copy_from_slice(&rest[offset + 6..offset + 12]);
            offset += 12;
        }
        let mut max_speed = 0;
        let mut overspeed = 0;
        let mut night_speed = 0;
        if attr & 0x02 != 0 {
            if rest.len() < offset + 5 {
                return Err(ProtocolError::InvalidParameters);
            }
            max_speed = u16::from_be_bytes(rest[offset..offset + 2].try_into().unwrap());
            overspeed = rest[offset + 2];
            night_speed = u16::from_be_bytes(rest[offset + 3..offset + 5].try_into().unwrap());
            offset += 5;
        }
        if rest.len() < offset + 2 {
            return Err(ProtocolError::InvalidParameters);
        }
        let name_len = u16::from_be_bytes(rest[offset..offset + 2].try_into().unwrap()) as usize;
        offset += 2;
        if rest.len() < offset + name_len {
            return Err(ProtocolError::InvalidParameters);
        }
        let (name, _, had_errors) = encoding_rs::GBK.decode(&rest[offset..offset + name_len]);
        if had_errors {
            return Err(ProtocolError::InvalidParameters);
        }
        areas.push(RectArea {
            id,
            attr,
            top_left_lat,
            top_left_lon,
            bottom_right_lat,
            bottom_right_lon,
            start,
            end,
            max_speed,
            overspeed,
            night_speed,
            name: name.into_owned(),
        });
        rest = &rest[offset + name_len..];
    }
    Ok((set_attr, areas))
}

/// 0x8401 phone book contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneContact {
    pub flag: u8,
    pub phone: String,
    pub name: String,
}

pub fn encode_phone_book(
    set_type: u8,
    contacts: &[PhoneContact],
) -> Result<Vec<u8>, ProtocolError> {
    if set_type > 3 || contacts.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = vec![set_type, contacts.len() as u8];
    for contact in contacts {
        let (phone, _, e1) = encoding_rs::GBK.encode(&contact.phone);
        let (name, _, e2) = encoding_rs::GBK.encode(&contact.name);
        if e1 || e2 || phone.len() > u8::MAX as usize || name.len() > u8::MAX as usize {
            return Err(ProtocolError::InvalidParameters);
        }
        body.push(contact.flag);
        body.push(phone.len() as u8);
        body.extend_from_slice(&phone);
        body.push(name.len() as u8);
        body.extend_from_slice(&name);
    }
    Ok(body)
}

pub fn decode_phone_book(body: &[u8]) -> Result<(u8, Vec<PhoneContact>), ProtocolError> {
    if body.len() < 2 {
        return Err(ProtocolError::InvalidParameters);
    }
    let set_type = body[0];
    let count = body[1] as usize;
    let mut rest = &body[2..];
    let mut contacts = Vec::with_capacity(count);
    for _ in 0..count {
        if rest.len() < 2 {
            return Err(ProtocolError::InvalidParameters);
        }
        let flag = rest[0];
        let phone_len = rest[1] as usize;
        if rest.len() < 2 + phone_len + 1 {
            return Err(ProtocolError::InvalidParameters);
        }
        let (phone, _, e1) = encoding_rs::GBK.decode(&rest[2..2 + phone_len]);
        let name_len = rest[2 + phone_len] as usize;
        if rest.len() < 3 + phone_len + name_len || e1 {
            return Err(ProtocolError::InvalidParameters);
        }
        let (name, _, e2) = encoding_rs::GBK.decode(&rest[3 + phone_len..3 + phone_len + name_len]);
        if e2 {
            return Err(ProtocolError::InvalidParameters);
        }
        contacts.push(PhoneContact {
            flag,
            phone: phone.into_owned(),
            name: name.into_owned(),
        });
        rest = &rest[3 + phone_len + name_len..];
    }
    Ok((set_type, contacts))
}

/// 0x8304 information service: type(u8) + length(u16 BE) + GBK content.
pub fn encode_info_service(info_type: u8, content: &str) -> Result<Vec<u8>, ProtocolError> {
    let (encoded, _, had_errors) = encoding_rs::GBK.encode(content);
    if had_errors || encoded.is_empty() || encoded.len() > 1024 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(3 + encoded.len());
    body.push(info_type);
    body.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
    body.extend_from_slice(&encoded);
    Ok(body)
}

pub fn decode_info_service(body: &[u8]) -> Result<(u8, String), ProtocolError> {
    if body.len() < 3 {
        return Err(ProtocolError::InvalidParameters);
    }
    let info_type = body[0];
    let len = u16::from_be_bytes([body[1], body[2]]) as usize;
    if len == 0 || len > 1024 || body.len() != 3 + len {
        return Err(ProtocolError::InvalidParameters);
    }
    let (decoded, _, had_errors) = encoding_rs::GBK.decode(&body[3..]);
    if had_errors {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok((info_type, decoded.into_owned()))
}

/// 0x8400 phone callback body: flag(0=normal call,1=monitor) + number (max 20 bytes).
pub fn encode_phone_call(flag: u8, phone: &str) -> Result<Vec<u8>, ProtocolError> {
    if flag > 1
        || phone.is_empty()
        || phone.len() > 20
        || !phone.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(phone.len() + 1);
    body.push(flag);
    body.extend_from_slice(phone.as_bytes());
    Ok(body)
}

pub fn decode_phone_call(body: &[u8]) -> Result<(u8, String), ProtocolError> {
    let Some((&flag, rest)) = body.split_first() else {
        return Err(ProtocolError::InvalidParameters);
    };
    if rest.is_empty() || rest.len() > 20 || !rest.iter().all(u8::is_ascii_digit) {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok((flag, String::from_utf8_lossy(rest).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_command_is_gbk_and_bounded() {
        let body = encode_text_command(1, "测试").expect("text");
        assert_eq!(body[0], 1);
        let (decoded, _, _) = encoding_rs::GBK.decode(&body[1..]);
        assert_eq!(decoded, "测试");
        assert!(encode_text_command(0, &"a".repeat(1025)).is_err());
    }

    #[test]
    fn parameter_and_attribute_queries_are_empty_bodies() {
        assert!(encode_query_all_parameters().is_empty());
        assert!(encode_query_terminal_attributes().is_empty());
    }
}

/// 0x8203 alarm acknowledgement: alarm message serial (WORD) + alarm type
/// bitmask (DWORD). The type bits are passed through unchanged; callers must
/// select the correct bits for the deployed terminal profile.
pub fn encode_alarm_ack(alarm_serial: u16, alarm_type: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(6);
    body.extend_from_slice(&alarm_serial.to_be_bytes());
    body.extend_from_slice(&alarm_type.to_be_bytes());
    body
}

#[cfg(test)]
mod alarm_tests {
    use super::*;

    #[test]
    fn alarm_ack_body_is_serial_plus_bitmask() {
        let body = encode_alarm_ack(7, 0x0000_0009);
        assert_eq!(body, vec![0x00, 0x07, 0x00, 0x00, 0x00, 0x09]);
    }

    #[test]
    fn phone_call_roundtrip_and_rejects_non_digits() {
        let body = encode_phone_call(1, "13800138000").expect("encode");
        assert_eq!(body[0], 1);
        assert_eq!(
            decode_phone_call(&body).unwrap(),
            (1, "13800138000".to_string())
        );
        assert!(encode_phone_call(1, "138-0013").is_err());
        assert!(decode_phone_call(&[1, b'x']).is_err());
    }

    #[test]
    fn info_service_roundtrip_and_bounds() {
        let body = encode_info_service(2, "欢迎乘车").expect("encode");
        assert_eq!(
            decode_info_service(&body).unwrap(),
            (2, "欢迎乘车".to_string())
        );
        assert!(decode_info_service(&body[..body.len() - 1]).is_err());
        assert!(encode_info_service(0, "").is_err());
    }

    #[test]
    fn candidate_event_question_and_menu_roundtrip() {
        let events = vec![EventItem {
            id: 1,
            event_type: 2,
            content: "超速".into(),
        }];
        assert_eq!(
            decode_event_set(&encode_event_set(&events).unwrap()).unwrap(),
            events
        );

        let question = Question {
            flag: 1,
            text: "继续行驶?".into(),
            answers: vec![(1, "是".into()), (2, "否".into())],
        };
        assert_eq!(
            decode_question(&encode_question(&question).unwrap()).unwrap(),
            question
        );

        let items = vec![InfoMenuItem {
            info_type: 2,
            name: "新闻".into(),
        }];
        assert_eq!(
            decode_info_menu(&encode_info_menu(1, &items).unwrap()).unwrap(),
            (1, items)
        );
    }

    #[test]
    fn phone_book_roundtrip_matches_2019_table() {
        let contacts = vec![PhoneContact {
            flag: 3,
            phone: "13800138000".into(),
            name: "张三".into(),
        }];
        let body = encode_phone_book(1, &contacts).unwrap();
        assert_eq!(body[0], 1);
        assert_eq!(body[1], 1);
        assert_eq!(decode_phone_book(&body).unwrap(), (1, contacts));
        assert!(encode_phone_book(4, &[]).is_err());
    }

    #[test]
    fn circle_area_and_delete_match_2019_tables() {
        let area = CircleArea {
            id: 7,
            attr: 0x03,
            lat: 31_230_000,
            lon: 121_470_000,
            radius: 500,
            start: [24, 1, 1, 8, 30, 0],
            end: [24, 12, 31, 18, 0, 0],
            max_speed: 80,
            overspeed: 10,
            night_speed: 60,
            name: "浦东".into(),
        };
        let body = encode_circle_areas(0, std::slice::from_ref(&area)).unwrap();
        assert_eq!(decode_circle_areas(&body).unwrap(), (0, vec![area]));
        let deleted = encode_delete_circle_areas(&[7, 9]).unwrap();
        assert_eq!(deleted[0], 2);
        assert_eq!(&deleted[1..5], &7u32.to_be_bytes());
    }

    #[test]
    fn rect_area_matches_2019_table() {
        let area = RectArea {
            id: 11,
            attr: 0x03,
            top_left_lat: 31_300_000,
            top_left_lon: 121_400_000,
            bottom_right_lat: 31_200_000,
            bottom_right_lon: 121_500_000,
            start: [24, 1, 1, 0, 0, 0],
            end: [24, 12, 31, 23, 59, 59],
            max_speed: 90,
            overspeed: 15,
            night_speed: 70,
            name: "矩形".into(),
        };
        let body = encode_rect_areas(1, std::slice::from_ref(&area)).unwrap();
        assert_eq!(decode_rect_areas(&body).unwrap(), (1, vec![area]));
    }

    #[test]
    fn polygon_area_matches_2019_tables() {
        let area = PolygonArea {
            id: 21,
            attr: 0x03,
            start: [24, 1, 1, 8, 0, 0],
            end: [24, 12, 31, 18, 0, 0],
            max_speed: 70,
            overspeed: 20,
            night_speed: 50,
            vertices: vec![
                (31_300_000, 121_400_000),
                (31_300_000, 121_500_000),
                (31_200_000, 121_450_000),
            ],
            name: "多边形".into(),
        };
        let body = encode_polygon_areas(0, std::slice::from_ref(&area)).unwrap();
        assert_eq!(decode_polygon_areas(&body).unwrap(), (0, vec![area]));
    }

    #[test]
    fn route_matches_2019_tables() {
        let point = RoutePoint {
            id: 1,
            segment_id: 2,
            lat: 31_300_000,
            lon: 121_400_000,
            width: 30,
            attr: 0x03,
            overlong: 600,
            under: 60,
            max_speed: 80,
            overspeed: 10,
            night_speed: 60,
        };
        let route = Route {
            id: 5,
            attr: 0x01,
            start: [24, 1, 1, 0, 0, 0],
            end: [24, 12, 31, 23, 59, 59],
            points: vec![point],
            name: "路线".into(),
        };
        let body = encode_route(&route).unwrap();
        assert_eq!(decode_route(&body).unwrap(), route);
        let deleted = encode_delete_routes(&[5]).unwrap();
        assert_eq!(deleted[0], 1);
    }

    #[test]
    fn area_query_matches_2019_table_60() {
        let query = AreaQuery {
            query_type: 1,
            ids: vec![7, 9],
        };
        let body = encode_area_query(&query).unwrap();
        assert_eq!(body[0], 1);
        assert_eq!(&body[1..5], &2u32.to_be_bytes());
        assert_eq!(decode_area_query(&body).unwrap(), query);
        assert!(decode_area_query(&body[..body.len() - 1]).is_err());
        assert!(encode_area_query(&AreaQuery {
            query_type: 5,
            ids: vec![]
        })
        .is_err());
    }

    #[test]
    fn tachograph_envelope_matches_2019_tables() {
        let command = TachographCommand {
            command: 0x01,
            data: vec![0xAA, 0xBB],
        };
        let body = encode_tachograph_command(&command);
        assert_eq!(decode_tachograph_command(&body).unwrap(), command);

        let upload = TachographUpload {
            serial: 7,
            command: 0x02,
            data: vec![0xCC],
        };
        let body = encode_tachograph_upload(&upload);
        assert_eq!(decode_tachograph_upload(&body).unwrap(), upload);
        assert!(decode_tachograph_upload(&[0, 1]).is_err());
    }

    #[test]
    fn multimedia_capture_messages_match_2019_tables() {
        let ack = MediaUploadAck {
            media_id: 9,
            retransmit: vec![1, 3],
        };
        assert_eq!(
            decode_media_upload_ack(&encode_media_upload_ack(&ack).unwrap()).unwrap(),
            ack
        );

        let capture = CameraCapture {
            channel: 1,
            command: 0xFFFF,
            interval: 5,
            save: 1,
            resolution: 2,
            quality: 5,
            contrast: 6,
            saturation: 7,
            chroma: 8,
        };
        assert_eq!(
            decode_camera_capture(&encode_camera_capture(&capture)).unwrap(),
            capture
        );

        let ack = CameraCaptureAck {
            serial: 7,
            result: 0,
            media_ids: vec![11, 12],
        };
        assert_eq!(
            decode_camera_capture_ack(&encode_camera_capture_ack(&ack).unwrap()).unwrap(),
            ack
        );
    }

    #[test]
    fn media_search_upload_and_transparent_match_2019_tables() {
        let search = MediaSearch {
            media_type: 0,
            channel: 0,
            event: 1,
            start: [24, 1, 1, 0, 0, 0],
            end: [24, 1, 2, 0, 0, 0],
        };
        assert_eq!(
            decode_media_search(&encode_media_search(&search)).unwrap(),
            search
        );
        let command = MediaUploadCommand {
            media_type: 2,
            channel: 1,
            event: 2,
            start: [24, 1, 1, 0, 0, 0],
            end: [24, 1, 2, 0, 0, 0],
            delete: 1,
        };
        assert_eq!(
            decode_media_upload_command(&encode_media_upload_command(&command)).unwrap(),
            command
        );
        let record = AudioRecord {
            command: 1,
            seconds: 30,
            save: 1,
            sample_rate: 2,
        };
        assert_eq!(
            decode_audio_record(&encode_audio_record(&record)).unwrap(),
            record
        );
        let single = SingleMediaUpload {
            media_id: 9,
            delete: 1,
        };
        assert_eq!(
            decode_single_media_upload(&encode_single_media_upload(&single)).unwrap(),
            single
        );
        assert_eq!(
            decode_transparent(&encode_transparent(0x41, &[1, 2])).unwrap(),
            (0x41, &[1u8, 2][..])
        );
    }

    #[test]
    fn media_search_ack_parses_basic_location_items() {
        let item = MediaSearchItem {
            media_id: 3,
            media_type: 0,
            channel: 1,
            event: 1,
            location: [7u8; 28],
        };
        let ack = MediaSearchAck {
            serial: 9,
            total: 1,
            items: vec![item],
        };
        let body = encode_media_search_ack(&ack).unwrap();
        assert_eq!(body.len(), 41);
        assert_eq!(decode_media_search_ack(&body).unwrap(), ack);
        assert!(decode_media_search_ack(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn area_query_ack_accepts_multiple_blocks() {
        let area = CircleArea {
            id: 1,
            attr: 0,
            lat: 0,
            lon: 0,
            radius: 1,
            start: [0; 6],
            end: [0; 6],
            max_speed: 0,
            overspeed: 0,
            night_speed: 0,
            name: String::new(),
        };
        let ack = AreaQueryAck {
            query_type: 1,
            total: 1,
            blocks: vec![AreaQueryBlock::Circle {
                set_attr: 0,
                areas: vec![area.clone()],
            }],
        };
        let body = encode_area_query_ack(&ack).unwrap();
        assert_eq!(decode_area_query_ack(&body).unwrap(), ack);
        let multi = AreaQueryAck {
            query_type: 1,
            total: 2,
            blocks: vec![
                AreaQueryBlock::Circle {
                    set_attr: 0,
                    areas: vec![area.clone()],
                },
                AreaQueryBlock::Circle {
                    set_attr: 0,
                    areas: vec![area],
                },
            ],
        };
        let body = encode_area_query_ack(&multi).unwrap();
        assert_eq!(decode_area_query_ack(&body).unwrap(), multi);
    }

    #[test]
    fn electronic_waybill_matches_2019_table_65() {
        let waybill = ElectronicWaybill {
            data: vec![1, 2, 3],
        };
        let body = encode_electronic_waybill(&waybill).unwrap();
        assert_eq!(&body[..4], &3u32.to_be_bytes());
        assert_eq!(decode_electronic_waybill(&body).unwrap(), waybill);
        assert!(decode_electronic_waybill(&body[..3]).is_err());
        assert!(encode_driver_id_request().is_empty());
    }

    #[test]
    fn driver_identity_matches_2019_table_66() {
        let identity = DriverIdentity {
            status: 1,
            time: [24, 1, 2, 3, 4, 5],
            card_result: 0,
            name: "张三".into(),
            qualification: "Q123".into(),
            agency: "交通局".into(),
            valid_date: [25, 12, 31, 0],
            id_card: "110101199001011234".into(),
            face_match: Some(96),
            uid: "UID123".into(),
        };
        let body = encode_driver_identity(&identity).unwrap();
        assert_eq!(decode_driver_identity(&body).unwrap(), identity);
        let absent = DriverIdentity {
            status: 2,
            time: [24, 1, 2, 3, 4, 5],
            card_result: 1,
            name: String::new(),
            qualification: String::new(),
            agency: String::new(),
            valid_date: [0; 4],
            id_card: String::new(),
            face_match: None,
            uid: String::new(),
        };
        let short = encode_driver_identity(&absent).unwrap();
        assert_eq!(short.len(), 8);
        assert_eq!(decode_driver_identity(&short).unwrap(), absent);
    }

    #[test]
    fn can_and_media_upload_match_2019_tables() {
        let can = CanUpload {
            time: [1, 2, 3, 4, 5],
            items: vec![CanItem {
                id: 0x18FF,
                data: [9; 8],
            }],
        };
        let body = encode_can_upload(&can).unwrap();
        assert_eq!(decode_can_upload(&body).unwrap(), can);

        let event = MediaEvent {
            media_id: 3,
            media_type: 0,
            format: 0,
            event: 4,
            channel: 1,
        };
        assert_eq!(
            decode_media_event(&encode_media_event(&event)).unwrap(),
            event
        );

        let media = MediaData {
            media_id: 4,
            media_type: 2,
            format: 4,
            event: 1,
            channel: 2,
            location: [7; 28],
            data: vec![1, 2, 3],
        };
        let body = encode_media_data(&media);
        assert_eq!(body.len(), 39);
        assert_eq!(decode_media_data(&body).unwrap(), media);
    }

    #[test]
    fn compressed_upload_and_rsa_key_match_2019_tables() {
        let raw = b"hello 808".to_vec();
        let gz = gzip_compress(&raw).unwrap();
        assert_eq!(gzip_decompress(&gz).unwrap(), raw);
        let upload = CompressedUpload { data: gz };
        let body = encode_compressed_upload(&upload).unwrap();
        assert_eq!(decode_compressed_upload(&body).unwrap(), upload);

        let key = RsaPublicKeyMessage {
            modulus: [7u8; 128],
        };
        let body = encode_rsa_public_key(&key);
        assert_eq!(body.len(), 132);
        assert_eq!(decode_rsa_public_key(&body).unwrap(), key);
    }

    #[test]
    fn media_search_ack_extended_parses_location_tlvs() {
        let ack = MediaSearchAckExtended {
            serial: 9,
            total: 1,
            item: MediaSearchItemExtended {
                media_id: 3,
                media_type: 0,
                channel: 1,
                event: 1,
                location: [7; 28],
                tlvs: vec![LocationTlv {
                    id: 0x01,
                    data: vec![1, 2, 3, 4],
                }],
            },
        };
        let body = encode_media_search_ack_extended(&ack).unwrap();
        assert_eq!(decode_media_search_ack_extended(&body).unwrap(), ack);
        assert!(decode_media_search_ack_extended(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn query_parameter_attribute_and_upgrade_bodies() {
        let response = TerminalParameterResponse {
            serial: 3,
            parameters: vec![TerminalParameter {
                id: 0x0001,
                value: vec![1, 2],
            }],
        };
        assert_eq!(
            decode_terminal_parameter_response(
                &encode_terminal_parameter_response(&response).unwrap()
            )
            .unwrap(),
            response
        );

        let mut model = [0u8; 30];
        model[..4].copy_from_slice(b"MDL1");
        let attrs = TerminalAttributeReport {
            terminal_type: 1,
            manufacturer: *b"ABCDE",
            model,
            terminal_id: *b"TERMID000000000000000000000001",
            iccid: [0; 10],
            hardware_version: "HW1".into(),
            firmware_version: "FW1".into(),
            gnss_attr: 1,
            comm_attr: 2,
        };
        assert_eq!(
            decode_terminal_attribute_report(&encode_terminal_attribute_report(&attrs).unwrap())
                .unwrap(),
            attrs
        );

        let result = UpgradeResult {
            upgrade_type: 0,
            result: 1,
        };
        assert_eq!(
            decode_upgrade_result(&encode_upgrade_result(&result).unwrap()).unwrap(),
            result
        );
    }

    #[test]
    fn vehicle_control_ack_carries_location_body() {
        let ack = VehicleControlAck {
            serial: 9,
            location: vec![0u8; 28],
        };
        assert_eq!(
            decode_vehicle_control_ack(&encode_vehicle_control_ack(&ack).unwrap()).unwrap(),
            ack
        );
        assert!(decode_vehicle_control_ack(&[0u8; 29]).is_err());
    }

    #[test]
    fn satellite_status_roundtrip_and_bounds() {
        let status = SatelliteStatus {
            beidou: vec![SatelliteState {
                id: 1,
                elevation: 45,
                azimuth: 180,
                cnr: 40,
            }],
            gps: vec![SatelliteState {
                id: 2,
                elevation: 30,
                azimuth: 270,
                cnr: 35,
            }],
            glonass: vec![],
            galileo: vec![SatelliteState {
                id: 3,
                elevation: 20,
                azimuth: 10,
                cnr: 30,
            }],
            location: [7u8; 28],
        };
        let encoded = encode_satellite_status(&status).unwrap();
        assert_eq!(decode_satellite_status(&encoded).unwrap(), status);
        let mut bad = encoded.clone();
        bad[2] = 91;
        assert!(decode_satellite_status(&bad).is_err());
    }

    #[test]
    fn tachograph_upload_carries_19056_record_block() {
        use crate::standards::gb19056::Gb19056RecordBlock;
        let block = Gb19056RecordBlock {
            mfmt: 0x21,
            sfmt: 0x01,
            data: vec![1, 2, 3],
        };
        let raw = block.encode().unwrap();
        let upload = TachographUpload {
            serial: 9,
            command: 0x21,
            data: raw,
        };
        let body = encode_tachograph_upload(&upload);
        let decoded = decode_tachograph_upload(&body).unwrap();
        assert_eq!(decoded.record_block().unwrap().mfmt, 0x21);
    }
}
