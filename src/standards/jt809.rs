//! JT/T 809-2019 transport-envelope skeleton.
//!
//! Wire layout implemented here:
//! `0x5B | body_len u32 | msg_sn u32 | msg_id u16 | encrypt u8 | compress u8 | body | crc32 u32 | 0x5D`
//!
//! Implemented here: transport envelope, login request body, login response
//! header, message-name catalog and a small handshake state machine. Encrypted
//! bodies, the full business catalog and regulator interoperability remain out
//! of scope; this is not a JT/T 809 conformance claim.

use std::{
    collections::BTreeMap,
    io::BufReader,
    sync::Arc,
    time::{Duration, Instant},
};

use crc32fast::Hasher;
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{
    rustls::{pki_types::ServerName, ClientConfig, RootCertStore},
    TlsConnector,
};

pub const JT809_START: u8 = 0x5b;
pub const JT809_END: u8 = 0x5d;

pub const MSG_LOGIN_REQUEST: u16 = 0x1001;
pub const MSG_LOGIN_RESPONSE: u16 = 0x1002;
pub const MSG_LOGOUT_REQUEST: u16 = 0x1003;
pub const MSG_LOGOUT_RESPONSE: u16 = 0x1004;
pub const MSG_LINKTEST_REQUEST: u16 = 0x1005;
pub const MSG_LINKTEST_RESPONSE: u16 = 0x1006;
pub const MSG_DISCONNECT_INFORM: u16 = 0x1007;
pub const MSG_CLOSE_LINK_INFORM: u16 = 0x1008;
pub const MSG_DOWN_CONNECT_REQUEST: u16 = 0x9001;
pub const MSG_DOWN_CONNECT_RESPONSE: u16 = 0x9002;
pub const MSG_DOWN_DISCONNECT_REQUEST: u16 = 0x9003;
pub const MSG_DOWN_DISCONNECT_RESPONSE: u16 = 0x9004;
pub const MSG_DOWN_LINKTEST_REQUEST: u16 = 0x9005;
pub const MSG_DOWN_LINKTEST_RESPONSE: u16 = 0x9006;
pub const MSG_DOWN_DISCONNECT_INFORM: u16 = 0x9007;
pub const MSG_DOWN_CLOSE_LINK_INFORM: u16 = 0x9008;

pub const MSG_UP_DYNAMIC: u16 = 0x1200;
pub const MSG_DOWN_DYNAMIC: u16 = 0x9200;
pub const MSG_UP_PLATFORM: u16 = 0x1300;
pub const MSG_DOWN_PLATFORM: u16 = 0x9300;
pub const MSG_UP_WARN: u16 = 0x1400;
pub const MSG_DOWN_WARN: u16 = 0x9400;
pub const MSG_UP_CTRL: u16 = 0x1500;
pub const MSG_DOWN_CTRL: u16 = 0x9500;
pub const MSG_UP_BASE: u16 = 0x1600;
pub const MSG_DOWN_BASE: u16 = 0x9600;

pub const SUB_REGISTER: u16 = 0x1201;
pub const SUB_REALTIME_LOCATION: u16 = 0x1202;
pub const SUB_LOCATION_SUPPLEMENT: u16 = 0x1203;
pub const SUB_LOCATION_REPLAY: u16 = SUB_REALTIME_LOCATION;
pub const SUB_RETURN_STARTUP_ACK: u16 = 0x1205;
pub const SUB_RETURN_END_ACK: u16 = 0x1206;
pub const SUB_APPLY_MONITOR: u16 = 0x1207;
pub const SUB_CANCEL_MONITOR: u16 = 0x1208;
pub const SUB_APPLY_HISTORY: u16 = 0x1209;
pub const SUB_ALARM_URGE_ACK: u16 = 0x1401;
pub const SUB_ALARM_URGE_REQ: u16 = 0x9401;
pub const SUB_ALARM_WARNING: u16 = 0x9402;
pub const SUB_ALARM_EXCHANGE: u16 = 0x9403;
pub const SUB_DRIVER_INFO_ACK: u16 = 0x120A;
pub const SUB_EWAYBILL_ACK: u16 = 0x120B;
pub const SUB_DRIVER_INFO: u16 = 0x120C;
pub const SUB_EWAYBILL: u16 = 0x120D;
pub const SUB_DRVLINE: u16 = 0x120E;

pub const SUB_DOWN_REGISTER_ACK: u16 = 0x9201;
pub const SUB_DOWN_LOCATION: u16 = 0x9202;
pub const SUB_DOWN_HISTORY: u16 = 0x9203;
pub const SUB_DOWN_CAR_INFO: u16 = 0x9204;
pub const SUB_DOWN_RETURN_STARTUP: u16 = 0x9205;
pub const SUB_DOWN_RETURN_END: u16 = 0x9206;
pub const SUB_DOWN_APPLY_MONITOR_ACK: u16 = 0x9207;
pub const SUB_DOWN_CANCEL_MONITOR_ACK: u16 = 0x9208;
pub const SUB_DOWN_APPLY_HISTORY_ACK: u16 = 0x9209;
pub const SUB_DOWN_DRIVER_INFO_REQ: u16 = 0x920A;
pub const SUB_DOWN_EWAYBILL_REQ: u16 = 0x920B;
pub const SUB_DOWN_DRVLINE_REQ: u16 = 0x920C;
pub const SUB_DOWN_DRVLINE_ACK: u16 = 0x920D;

pub const MSG_PLATFORM_ACK: u16 = 0x9001;

pub fn message_name(msg_id: u16) -> &'static str {
    match msg_id {
        0x1001 => "platform_login_request",
        0x1002 => "platform_login_response",
        0x1003 => "platform_logout_request",
        0x1004 => "platform_logout_response",
        0x1005 => "main_linktest_request",
        0x1006 => "main_linktest_response",
        0x1007 => "main_disconnect_inform",
        0x1008 => "close_link_inform",
        0x9001 => "down_connect_request",
        0x9002 => "down_connect_response",
        0x9003 => "down_disconnect_request",
        0x9004 => "down_disconnect_response",
        0x9005 => "down_linktest_request",
        0x9006 => "down_linktest_response",
        0x9007 => "down_disconnect_inform",
        0x9008 => "down_close_link_inform",
        0x1200 => "up_dynamic_info",
        0x9200 => "down_dynamic_info",
        0x1300 => "up_platform_msg",
        0x9300 => "down_platform_msg",
        0x1400 => "up_warn_msg",
        0x9400 => "down_warn_msg",
        0x1500 => "up_ctrl_msg",
        0x9500 => "down_ctrl_msg",
        0x1600 => "up_base_msg",
        0x9600 => "down_base_msg",
        0x1201 => "up_register",
        0x1202 => "up_realtime_location",
        0x1203 => "up_history_location",
        0x120C => "up_driver_info",
        0x120D => "up_ewaybill",
        0x120E => "up_drvline",
        0x9201 => "down_register_ack",
        0x9202 => "down_location",
        0x9203 => "down_history_location",
        0x9204 => "down_car_info",
        0x9205 => "down_return_startup",
        0x9206 => "down_return_end",
        0x920C => "down_drvline_req",
        0x920D => "down_drvline_ack",
        _ => "unknown",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Jt809SessionState {
    #[default]
    Disconnected,
    LoginSent,
    Online,
    Closed,
}

#[derive(Debug, Default)]
pub struct Jt809Session {
    state: Jt809SessionState,
}

impl Jt809Session {
    pub fn new() -> Self {
        Self {
            state: Jt809SessionState::Disconnected,
        }
    }

    pub fn state(&self) -> Jt809SessionState {
        self.state
    }

    pub fn mark_login_sent(&mut self) -> Result<(), Jt809Error> {
        if self.state != Jt809SessionState::Disconnected {
            return Err(Jt809Error::State);
        }
        self.state = Jt809SessionState::LoginSent;
        Ok(())
    }

    pub fn on_login_response(&mut self, result: u8) -> Result<(), Jt809Error> {
        if self.state != Jt809SessionState::LoginSent {
            return Err(Jt809Error::State);
        }
        self.state = if result == 0 {
            Jt809SessionState::Online
        } else {
            Jt809SessionState::Disconnected
        };
        Ok(())
    }

    pub fn close(&mut self) {
        self.state = Jt809SessionState::Closed;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809Frame {
    pub msg_id: u16,
    pub msg_sn: u32,
    pub encrypt: u8,
    pub compress: u8,
    pub body: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Jt809Error {
    #[error("missing 0x5B/0x5D delimiters")]
    Delimiter,
    #[error("frame is too short")]
    TooShort,
    #[error("length mismatch")]
    Length,
    #[error("crc32 mismatch")]
    Crc,
    #[error("invalid login body")]
    LoginBody,
    #[error("invalid login response")]
    LoginResponse,
    #[error("invalid session state")]
    State,
    #[error("encrypted 809 body is not implemented")]
    Encrypted,
    #[error("compressed 809 body is not implemented")]
    Compressed,
    #[error("invalid 809 business body")]
    BusinessBody,
    #[error("invalid 809 location body")]
    LocationBody,
    #[error("invalid 809 ack body")]
    AckBody,
    #[error("invalid 809 alarm body")]
    AlarmBody,
    #[error("809 connection closed")]
    Eof,
    #[error("809 io error: {0}")]
    Io(String),
    #[error("809 login rejected with result {0}")]
    LoginRejected(u8),
}

impl Jt809Frame {
    pub fn encode(&self) -> Vec<u8> {
        let body_len = 4 + 2 + 1 + 1 + self.body.len();
        let mut out = Vec::with_capacity(body_len + 11);
        out.push(JT809_START);
        out.extend_from_slice(&(body_len as u32).to_be_bytes());
        out.extend_from_slice(&self.msg_sn.to_be_bytes());
        out.extend_from_slice(&self.msg_id.to_be_bytes());
        out.push(self.encrypt);
        out.push(self.compress);
        out.extend_from_slice(&self.body);
        let mut crc = Hasher::new();
        crc.update(&out[1..]);
        out.extend_from_slice(&crc.finalize().to_be_bytes());
        out.push(JT809_END);
        out
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Jt809Error> {
        if raw.len() < 13 || raw.first() != Some(&JT809_START) || raw.last() != Some(&JT809_END) {
            return Err(Jt809Error::Delimiter);
        }
        let body_len = u32::from_be_bytes([raw[1], raw[2], raw[3], raw[4]]) as usize;
        if body_len < 8 || raw.len() != 1 + 4 + body_len + 4 + 1 {
            return Err(Jt809Error::Length);
        }
        let crc_offset = 1 + 4 + body_len;
        let expected = u32::from_be_bytes([
            raw[crc_offset],
            raw[crc_offset + 1],
            raw[crc_offset + 2],
            raw[crc_offset + 3],
        ]);
        let mut crc = Hasher::new();
        crc.update(&raw[1..crc_offset]);
        if crc.finalize() != expected {
            return Err(Jt809Error::Crc);
        }
        let msg_sn = u32::from_be_bytes([raw[5], raw[6], raw[7], raw[8]]);
        let msg_id = u16::from_be_bytes([raw[9], raw[10]]);
        let encrypt = raw[11];
        let compress = raw[12];
        if encrypt != 0 {
            return Err(Jt809Error::Encrypted);
        }
        if compress != 0 {
            return Err(Jt809Error::Compressed);
        }
        let body = raw[13..1 + 4 + body_len].to_vec();
        Ok(Self {
            msg_id,
            msg_sn,
            encrypt,
            compress,
            body,
        })
    }
}

/// Full JT/T 809-2019 frame using the 30-byte header, 0x5B/0x5D escaping and
/// CRC32 over the unescaped header+body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809Frame2019 {
    pub msg_sn: u32,
    pub msg_id: u16,
    pub center_id: u32,
    pub version: [u8; 3],
    pub encrypt: u8,
    pub encrypt_key: u32,
    pub time: u64,
    pub body: Vec<u8>,
}

impl Jt809Frame2019 {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.encrypt != 0 {
            return Err(Jt809Error::Encrypted);
        }
        self.encode_with_cipher(None)
    }

    /// Encode with optional body encryption. `encrypt == 1` requires a cipher.
    pub fn encode_with_cipher(&self, cipher: Option<&Jt809Cipher>) -> Result<Vec<u8>, Jt809Error> {
        if self.encrypt == 1 && cipher.is_none() {
            return Err(Jt809Error::Encrypted);
        }
        if self.encrypt != 0 && self.encrypt != 1 {
            return Err(Jt809Error::Encrypted);
        }
        let msg_length = (1 + Jt809Header2019::LEN + self.body.len() + 4 + 1) as u32;
        let header = Jt809Header2019 {
            msg_length,
            msg_sn: self.msg_sn,
            msg_id: self.msg_id,
            center_id: self.center_id,
            version: self.version,
            encrypt: self.encrypt,
            encrypt_key: self.encrypt_key,
            time: self.time,
        }
        .encode();
        let mut inner = Vec::with_capacity(Jt809Header2019::LEN + self.body.len() + 4);
        inner.extend_from_slice(&header);
        inner.extend_from_slice(&self.body);
        if self.encrypt == 1 {
            cipher
                .expect("checked above")
                .apply(self.encrypt_key, &mut inner[Jt809Header2019::LEN..]);
        }
        let mut crc = Hasher::new();
        crc.update(&inner);
        inner.extend_from_slice(&crc.finalize().to_be_bytes());
        let mut out = Vec::with_capacity(inner.len() + 2);
        out.push(JT809_START);
        jt809_escape_into(&inner, &mut out);
        out.push(JT809_END);
        Ok(out)
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Jt809Error> {
        Self::decode_with_cipher(raw, None)
    }

    pub fn decode_with_cipher(
        raw: &[u8],
        cipher: Option<&Jt809Cipher>,
    ) -> Result<Self, Jt809Error> {
        if raw.len() < 7 || raw.first() != Some(&JT809_START) || raw.last() != Some(&JT809_END) {
            return Err(Jt809Error::Delimiter);
        }
        let inner = jt809_unescape(&raw[1..raw.len() - 1])?;
        if inner.len() < Jt809Header2019::LEN + 4 {
            return Err(Jt809Error::TooShort);
        }
        let header = Jt809Header2019::decode(&inner[..Jt809Header2019::LEN])?;
        if header.msg_length as usize != inner.len() + 2 {
            return Err(Jt809Error::Length);
        }
        let crc_at = inner.len() - 4;
        let expected = u32::from_be_bytes(inner[crc_at..].try_into().unwrap());
        let mut crc = Hasher::new();
        crc.update(&inner[..crc_at]);
        if crc.finalize() != expected {
            return Err(Jt809Error::Crc);
        }
        let mut body = inner[Jt809Header2019::LEN..crc_at].to_vec();
        if header.encrypt == 1 {
            let cipher = cipher.ok_or(Jt809Error::Encrypted)?;
            cipher.apply(header.encrypt_key, &mut body);
        }
        Ok(Self {
            msg_sn: header.msg_sn,
            msg_id: header.msg_id,
            center_id: header.center_id,
            version: header.version,
            encrypt: header.encrypt,
            encrypt_key: header.encrypt_key,
            time: header.time,
            body,
        })
    }
}

fn jt809_escape_into(data: &[u8], out: &mut Vec<u8>) {
    out.reserve(data.len());
    for byte in data {
        match byte {
            0x5b => out.extend_from_slice(&[0x5a, 0x01]),
            0x5d => out.extend_from_slice(&[0x5a, 0x02]),
            0x5a => out.extend_from_slice(&[0x5a, 0x03]),
            0x5e => out.extend_from_slice(&[0x5a, 0x04]),
            _ => out.push(*byte),
        }
    }
}

fn jt809_unescape(data: &[u8]) -> Result<Vec<u8>, Jt809Error> {
    let mut out = Vec::with_capacity(data.len());
    let mut index = 0;
    while index < data.len() {
        if data[index] == 0x5a {
            let code = *data.get(index + 1).ok_or(Jt809Error::Delimiter)?;
            out.push(match code {
                0x01 => 0x5b,
                0x02 => 0x5d,
                0x03 => 0x5a,
                0x04 => 0x5e,
                _ => return Err(Jt809Error::Delimiter),
            });
            index += 2;
        } else {
            out.push(data[index]);
            index += 1;
        }
    }
    Ok(out)
}

/// JT/T 809-2019 30-byte message header (tables 5-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809Header2019 {
    /// Total packet length including 0x5B, header, body, CRC and 0x5D.
    pub msg_length: u32,
    pub msg_sn: u32,
    pub msg_id: u16,
    pub center_id: u32,
    pub version: [u8; 3],
    pub encrypt: u8,
    pub encrypt_key: u32,
    /// UTC seconds.
    pub time: u64,
}

impl Jt809Header2019 {
    pub const LEN: usize = 30;

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut out = [0u8; Self::LEN];
        out[0..4].copy_from_slice(&self.msg_length.to_be_bytes());
        out[4..8].copy_from_slice(&self.msg_sn.to_be_bytes());
        out[8..10].copy_from_slice(&self.msg_id.to_be_bytes());
        out[10..14].copy_from_slice(&self.center_id.to_be_bytes());
        out[14..17].copy_from_slice(&self.version);
        out[17] = self.encrypt;
        out[18..22].copy_from_slice(&self.encrypt_key.to_be_bytes());
        out[22..30].copy_from_slice(&self.time.to_be_bytes());
        out
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Jt809Error> {
        if raw.len() != Self::LEN {
            return Err(Jt809Error::TooShort);
        }
        let version: [u8; 3] = raw[14..17].try_into().map_err(|_| Jt809Error::TooShort)?;
        if raw[17] != 0 && raw[17] != 1 {
            return Err(Jt809Error::Encrypted);
        }
        Ok(Self {
            msg_length: u32::from_be_bytes(raw[0..4].try_into().unwrap()),
            msg_sn: u32::from_be_bytes(raw[4..8].try_into().unwrap()),
            msg_id: u16::from_be_bytes(raw[8..10].try_into().unwrap()),
            center_id: u32::from_be_bytes(raw[10..14].try_into().unwrap()),
            version,
            encrypt: raw[17],
            encrypt_key: u32::from_be_bytes(raw[18..22].try_into().unwrap()),
            time: u64::from_be_bytes(raw[22..30].try_into().unwrap()),
        })
    }
}

/// Login message body subset: time[6] + serial u16 + user[24] + password[24]
/// 2019 table 11 main-link login request (50 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809Login {
    pub user_id: u32,
    pub password: String,
    pub center_id: u32,
    pub down_link_ip: String,
    pub down_link_port: u16,
}

impl Jt809Login {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let password = fixed_ascii_width(&self.password, 8)?;
        let ip = fixed_ascii_width(&self.down_link_ip, 32)?;
        let mut out = Vec::with_capacity(50);
        out.extend_from_slice(&self.user_id.to_be_bytes());
        out.extend_from_slice(&password);
        out.extend_from_slice(&self.center_id.to_be_bytes());
        out.extend_from_slice(&ip);
        out.extend_from_slice(&self.down_link_port.to_be_bytes());
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 50 {
            return Err(Jt809Error::LoginBody);
        }
        Ok(Self {
            user_id: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
            password: trim_ascii(&body[4..12])?,
            center_id: u32::from_be_bytes([body[12], body[13], body[14], body[15]]),
            down_link_ip: trim_ascii(&body[16..48])?,
            down_link_port: u16::from_be_bytes([body[48], body[49]]),
        })
    }
}

fn fixed_ascii_width(value: &str, width: usize) -> Result<Vec<u8>, Jt809Error> {
    if value.len() > width || !value.is_ascii() {
        return Err(Jt809Error::LoginBody);
    }
    let mut out = vec![0u8; width];
    out[..value.len()].copy_from_slice(value.as_bytes());
    Ok(out)
}

/// 0x1007 main-link disconnect notice: error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809DisconnectNotice {
    pub error_code: u8,
}

/// 0x1008 subordinate link close notice: reason code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809CloseLinkNotice {
    pub reason: u8,
}

/// 0x9001 subordinate-link connect request: verification code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809DownConnectRequest {
    pub verify_code: u32,
}

impl Jt809DisconnectNotice {
    pub fn encode(&self) -> Vec<u8> {
        vec![self.error_code]
    }
    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 1 {
            return Err(Jt809Error::LoginBody);
        }
        Ok(Self {
            error_code: body[0],
        })
    }
}

impl Jt809CloseLinkNotice {
    pub fn encode(&self) -> Vec<u8> {
        vec![self.reason]
    }
    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 1 {
            return Err(Jt809Error::LoginBody);
        }
        Ok(Self { reason: body[0] })
    }
}

impl Jt809DownConnectRequest {
    pub fn encode(&self) -> Vec<u8> {
        self.verify_code.to_be_bytes().to_vec()
    }
    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 4 {
            return Err(Jt809Error::LoginBody);
        }
        Ok(Self {
            verify_code: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809Logout {
    pub user_id: u32,
    pub password: String,
}

impl Jt809Logout {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let password = fixed_ascii_width(&self.password, 8)?;
        let mut out = Vec::with_capacity(12);
        out.extend_from_slice(&self.user_id.to_be_bytes());
        out.extend_from_slice(&password);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 12 {
            return Err(Jt809Error::LoginBody);
        }
        Ok(Self {
            user_id: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
            password: trim_ascii(&body[4..12])?,
        })
    }
}

fn encode_control_body(
    vehicle_no: &str,
    color: u8,
    subtype: u16,
    data: &[u8],
) -> Result<Vec<u8>, Jt809Error> {
    let (plate, _, bad) = encoding_rs::GBK.encode(vehicle_no);
    if bad || plate.len() > 21 || data.len() > u32::MAX as usize {
        return Err(Jt809Error::BusinessBody);
    }
    let mut out = Vec::with_capacity(28 + data.len());
    let mut fixed = [0u8; 21];
    fixed[..plate.len()].copy_from_slice(&plate);
    out.extend_from_slice(&fixed);
    out.push(color);
    out.extend_from_slice(&subtype.to_be_bytes());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    Ok(out)
}

fn decode_control_body(body: &[u8], subtype: u16) -> Result<(String, u8, &[u8]), Jt809Error> {
    if body.len() < 28 {
        return Err(Jt809Error::BusinessBody);
    }
    if u16::from_be_bytes([body[22], body[23]]) != subtype {
        return Err(Jt809Error::BusinessBody);
    }
    let len = u32::from_be_bytes([body[24], body[25], body[26], body[27]]) as usize;
    if body.len() != 28 + len {
        return Err(Jt809Error::BusinessBody);
    }
    let end = body[..21].iter().position(|b| *b == 0).unwrap_or(21);
    let (plate, _, bad) = encoding_rs::GBK.decode(&body[..end]);
    if bad {
        return Err(Jt809Error::BusinessBody);
    }
    Ok((plate.into_owned(), body[21], &body[28..]))
}

fn fixed_ascii_pad(value: &str, size: usize) -> Result<Vec<u8>, Jt809Error> {
    if !value.is_ascii() || value.len() > size {
        return Err(Jt809Error::BusinessBody);
    }
    let mut out = vec![0u8; size];
    out[..value.len()].copy_from_slice(value.as_bytes());
    Ok(out)
}

/// 0x9601 request missing vehicle static information (8.3.7.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809VehicleStaticRequest {
    pub vehicle_no: String,
    pub color: u8,
}

impl Jt809VehicleStaticRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, 0x9601, &[])
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9601)?;
        if !data.is_empty() {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self { vehicle_no, color })
    }
}

/// Driver identity fields added by JT/T 809-2019 amendment 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DriverIdentity {
    pub drive_name: String,
    pub driver_id: String,
    pub licence: String,
    pub org_name: String,
    pub valid_date: u64,
    pub match_score: u8,
    pub driver_uid: String,
}

impl Jt809DriverIdentity {
    fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut out = Vec::with_capacity(305);
        for (value, width) in [
            (&self.drive_name, 16usize),
            (&self.driver_id, 20),
            (&self.licence, 40),
            (&self.org_name, 200),
        ] {
            let (bytes, _, bad) = encoding_rs::GBK.encode(value);
            if bad || bytes.len() > width {
                return Err(Jt809Error::BusinessBody);
            }
            let mut field = [0u8; 200];
            field[..bytes.len()].copy_from_slice(&bytes);
            out.extend_from_slice(&field[..width]);
        }
        out.extend_from_slice(&self.valid_date.to_be_bytes());
        out.push(self.match_score);
        let (uid, _, bad) = encoding_rs::GBK.encode(&self.driver_uid);
        if bad || uid.len() > 20 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut field = [0u8; 20];
        field[..uid.len()].copy_from_slice(&uid);
        out.extend_from_slice(&field);
        Ok(out)
    }

    fn decode(data: &[u8]) -> Result<Self, Jt809Error> {
        if data.len() != 305 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut fields = Vec::with_capacity(5);
        let mut offset = 0usize;
        for width in [16usize, 20, 40, 200] {
            let slice = &data[offset..offset + width];
            let end = slice.iter().position(|b| *b == 0).unwrap_or(width);
            let (text, _, bad) = encoding_rs::GBK.decode(&slice[..end]);
            if bad {
                return Err(Jt809Error::BusinessBody);
            }
            fields.push(text.into_owned());
            offset += width;
        }
        let valid_date = u64::from_be_bytes(data[offset..offset + 8].try_into().unwrap());
        offset += 8;
        let match_score = data[offset];
        offset += 1;
        let slice = &data[offset..offset + 20];
        let end = slice.iter().position(|b| *b == 0).unwrap_or(20);
        let (uid, _, bad) = encoding_rs::GBK.decode(&slice[..end]);
        if bad {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            drive_name: fields.remove(0),
            driver_id: fields.remove(0),
            licence: fields.remove(0),
            org_name: fields.remove(0),
            valid_date,
            match_score,
            driver_uid: uid.into_owned(),
        })
    }
}

/// 0x1401 alarm supervision ACK (table 66).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809AlarmUrgeAck {
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub result: u8,
}

impl Jt809AlarmUrgeAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.result > 3 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut out = Vec::with_capacity(13);
        out.extend_from_slice(&SUB_ALARM_URGE_ACK.to_be_bytes());
        out.extend_from_slice(&7u32.to_be_bytes());
        out.extend_from_slice(&self.source_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out.push(self.result);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 13
            || u16::from_be_bytes([body[0], body[1]]) != SUB_ALARM_URGE_ACK
            || u32::from_be_bytes([body[2], body[3], body[4], body[5]]) != 7
            || body[12] > 3
        {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            source_type: u16::from_be_bytes([body[6], body[7]]),
            source_msg_sn: u32::from_be_bytes([body[8], body[9], body[10], body[11]]),
            result: body[12],
        })
    }
}

/// 0x9402 alarm warning / advisory (table 69).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809AlarmWarning {
    pub platform_id: [u8; 11],
    pub warn_type: u16,
    pub warn_time: u64,
    pub start_time: u64,
    pub end_time: u64,
    pub vehicle_no: String,
    pub vehicle_color: u8,
    pub target_platform_id: [u8; 11],
    pub drvline_id: u32,
    pub content: Vec<u8>,
}

impl Jt809AlarmWarning {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        self.encode_as(SUB_ALARM_WARNING)
    }

    /// 0x9403 real-time cross-platform alarm exchange (table 70).
    pub fn encode_realtime(&self) -> Result<Vec<u8>, Jt809Error> {
        self.encode_as(SUB_ALARM_EXCHANGE)
    }

    fn encode_as(&self, subtype: u16) -> Result<Vec<u8>, Jt809Error> {
        if self.content.len() > 1024 {
            return Err(Jt809Error::BusinessBody);
        }
        let (plate, _, bad) = encoding_rs::GBK.encode(&self.vehicle_no);
        if bad || plate.len() > 21 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = Vec::with_capacity(78 + self.content.len());
        data.extend_from_slice(&self.platform_id);
        data.extend_from_slice(&self.warn_type.to_be_bytes());
        data.extend_from_slice(&self.warn_time.to_be_bytes());
        data.extend_from_slice(&self.start_time.to_be_bytes());
        data.extend_from_slice(&self.end_time.to_be_bytes());
        let mut fixed = [0u8; 21];
        fixed[..plate.len()].copy_from_slice(&plate);
        data.extend_from_slice(&fixed);
        data.push(self.vehicle_color);
        data.extend_from_slice(&self.target_platform_id);
        data.extend_from_slice(&self.drvline_id.to_be_bytes());
        data.extend_from_slice(&(self.content.len() as u32).to_be_bytes());
        data.extend_from_slice(&self.content);
        let mut out = Vec::with_capacity(6 + data.len());
        out.extend_from_slice(&subtype.to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&data);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        Self::decode_as(body, SUB_ALARM_WARNING)
    }

    pub fn decode_realtime(body: &[u8]) -> Result<Self, Jt809Error> {
        Self::decode_as(body, SUB_ALARM_EXCHANGE)
    }

    fn decode_as(body: &[u8], subtype: u16) -> Result<Self, Jt809Error> {
        if body.len() < 6 + 78 || u16::from_be_bytes([body[0], body[1]]) != subtype {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = u32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        if body.len() != 6 + data_len || data_len < 78 {
            return Err(Jt809Error::BusinessBody);
        }
        let data = &body[6..];
        let content_len = u32::from_be_bytes([data[74], data[75], data[76], data[77]]) as usize;
        if content_len > 1024 || data.len() != 78 + content_len {
            return Err(Jt809Error::BusinessBody);
        }
        let end = data[37..58].iter().position(|b| *b == 0).unwrap_or(21);
        let (plate, _, bad) = encoding_rs::GBK.decode(&data[37..37 + end]);
        if bad {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            platform_id: data[0..11].try_into().unwrap(),
            warn_type: u16::from_be_bytes([data[11], data[12]]),
            warn_time: u64::from_be_bytes(data[13..21].try_into().unwrap()),
            start_time: u64::from_be_bytes(data[21..29].try_into().unwrap()),
            end_time: u64::from_be_bytes(data[29..37].try_into().unwrap()),
            vehicle_no: plate.into_owned(),
            vehicle_color: data[58],
            target_platform_id: data[59..70].try_into().unwrap(),
            drvline_id: u32::from_be_bytes(data[70..74].try_into().unwrap()),
            content: data[78..].to_vec(),
        })
    }
}

/// 0x9401 alarm supervision request (table 68).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809AlarmUrgeRequest {
    pub platform_id: [u8; 11],
    pub warn_time: u64,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub supervision_deadline: u64,
    pub supervision_level: u8,
    pub supervisor: String,
    pub supervisor_tel: String,
    pub supervisor_email: String,
}

impl Jt809AlarmUrgeRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.supervision_level > 1 {
            return Err(Jt809Error::BusinessBody);
        }
        let supervisor = fixed_gbk(&self.supervisor, 16)?;
        let tel = fixed_gbk(&self.supervisor_tel, 20)?;
        let email = fixed_gbk(&self.supervisor_email, 32)?;
        let mut out = Vec::with_capacity(108);
        out.extend_from_slice(&SUB_ALARM_URGE_REQ.to_be_bytes());
        out.extend_from_slice(&102u32.to_be_bytes());
        out.extend_from_slice(&self.platform_id);
        out.extend_from_slice(&self.warn_time.to_be_bytes());
        out.extend_from_slice(&self.source_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out.extend_from_slice(&self.supervision_deadline.to_be_bytes());
        out.push(self.supervision_level);
        out.extend_from_slice(&supervisor);
        out.extend_from_slice(&tel);
        out.extend_from_slice(&email);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 108
            || u16::from_be_bytes([body[0], body[1]]) != SUB_ALARM_URGE_REQ
            || u32::from_be_bytes([body[2], body[3], body[4], body[5]]) != 102
        {
            return Err(Jt809Error::BusinessBody);
        }
        let data = &body[6..];
        if data[33] > 1 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            platform_id: data[0..11].try_into().unwrap(),
            warn_time: u64::from_be_bytes(data[11..19].try_into().unwrap()),
            source_type: u16::from_be_bytes([data[19], data[20]]),
            source_msg_sn: u32::from_be_bytes([data[21], data[22], data[23], data[24]]),
            supervision_deadline: u64::from_be_bytes(data[25..33].try_into().unwrap()),
            supervision_level: data[33],
            supervisor: read_gbk(&data[34..50])?,
            supervisor_tel: read_gbk(&data[50..70])?,
            supervisor_email: read_gbk(&data[70..102])?,
        })
    }
}

fn fixed_gbk(value: &str, width: usize) -> Result<Vec<u8>, Jt809Error> {
    let (bytes, _, bad) = encoding_rs::GBK.encode(value);
    if bad || bytes.len() > width {
        return Err(Jt809Error::BusinessBody);
    }
    let mut out = vec![0u8; width];
    out[..bytes.len()].copy_from_slice(&bytes);
    Ok(out)
}

fn read_gbk(field: &[u8]) -> Result<String, Jt809Error> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    let (text, _, bad) = encoding_rs::GBK.decode(&field[..end]);
    if bad {
        return Err(Jt809Error::BusinessBody);
    }
    Ok(text.into_owned())
}

/// 0x920A driver-info request (table 50): upload flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DriverInfoRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub flag: u8,
}

impl Jt809DriverInfoRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.flag > 1 {
            return Err(Jt809Error::BusinessBody);
        }
        encode_control_body(&self.vehicle_no, self.color, 0x920A, &[self.flag])
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x920A)?;
        if data.len() != 1 || data[0] > 1 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            flag: data[0],
        })
    }
}

/// 0x920B e-waybill request (table 51), empty data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809EWaybillRequest {
    pub vehicle_no: String,
    pub color: u8,
}

impl Jt809EWaybillRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, 0x920B, &[])
    }
    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x920B)?;
        if !data.is_empty() {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self { vehicle_no, color })
    }
}

/// 0x920C route request (table 52), empty data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DrvLineRequest {
    pub vehicle_no: String,
    pub color: u8,
}

impl Jt809DrvLineRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, 0x920C, &[])
    }
    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x920C)?;
        if !data.is_empty() {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self { vehicle_no, color })
    }
}

/// 0x920D route ACK (table 53).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DrvLineAckDown {
    pub vehicle_no: String,
    pub color: u8,
    pub drvline_id: u32,
    pub result: u8,
    pub reason: String,
}

impl Jt809DrvLineAckDown {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.result > 3 {
            return Err(Jt809Error::BusinessBody);
        }
        let (reason, _, bad) = encoding_rs::GBK.encode(&self.reason);
        if bad || reason.len() > u16::MAX as usize {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = Vec::with_capacity(7 + reason.len());
        data.extend_from_slice(&self.drvline_id.to_be_bytes());
        data.push(self.result);
        data.extend_from_slice(&(reason.len() as u16).to_be_bytes());
        data.extend_from_slice(&reason);
        encode_control_body(&self.vehicle_no, self.color, 0x920D, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x920D)?;
        if data.len() < 7 || data[4] > 3 {
            return Err(Jt809Error::BusinessBody);
        }
        let len = u16::from_be_bytes([data[5], data[6]]) as usize;
        if data.len() != 7 + len {
            return Err(Jt809Error::BusinessBody);
        }
        let (reason, _, bad) = encoding_rs::GBK.decode(&data[7..]);
        if bad {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            drvline_id: u32::from_be_bytes(data[0..4].try_into().unwrap()),
            result: data[4],
            reason: reason.into_owned(),
        })
    }
}

/// 0x9202 downlink location exchange (table 41).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809LocationDown {
    pub vehicle_no: String,
    pub color: u8,
    pub gnss: Jt809GnssBody,
}

impl Jt809LocationDown {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, 0x9202, &self.gnss.encode()?)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9202)?;
        let (gnss, used) = Jt809GnssBody::decode(data)?;
        if used != data.len() {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            gnss,
        })
    }
}

/// 0x9203 downlink history location exchange (table 42): 1..=5 GNSS bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809HistoryLocationDown {
    pub vehicle_no: String,
    pub color: u8,
    pub items: Vec<Jt809GnssBody>,
}

impl Jt809HistoryLocationDown {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.items.is_empty() || self.items.len() > 5 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = vec![self.items.len() as u8];
        for item in &self.items {
            data.extend_from_slice(&item.encode()?);
        }
        encode_control_body(&self.vehicle_no, self.color, 0x9203, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9203)?;
        if data.is_empty() {
            return Err(Jt809Error::BusinessBody);
        }
        let count = data[0] as usize;
        if !(1..=5).contains(&count) {
            return Err(Jt809Error::BusinessBody);
        }
        let mut offset = 1usize;
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            let (item, used) = Jt809GnssBody::decode(&data[offset..])?;
            offset += used;
            items.push(item);
        }
        if offset != data.len() {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            items,
        })
    }
}

/// 0x9201 vehicle registration ACK (table 40): no vehicle-number control body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809RegisterAck {
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub result: u8,
}

impl Jt809RegisterAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.result > 3 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut out = Vec::with_capacity(13);
        out.extend_from_slice(&0x9201u16.to_be_bytes());
        out.extend_from_slice(&7u32.to_be_bytes());
        out.extend_from_slice(&self.source_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out.push(self.result);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 13
            || u16::from_be_bytes([body[0], body[1]]) != 0x9201
            || u32::from_be_bytes([body[2], body[3], body[4], body[5]]) != 7
            || body[12] > 3
        {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            source_type: u16::from_be_bytes([body[6], body[7]]),
            source_msg_sn: u32::from_be_bytes([body[8], body[9], body[10], body[11]]),
            result: body[12],
        })
    }
}

/// 0x9204 downlink vehicle static information (table 44).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809VehicleStaticDown {
    pub vehicle_no: String,
    pub color: u8,
    pub car_info: Vec<u8>,
}

impl Jt809VehicleStaticDown {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, 0x9204, &self.car_info)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9204)?;
        Ok(Self {
            vehicle_no,
            color,
            car_info: data.to_vec(),
        })
    }
}

/// 0x9207/0x9208/0x9209 monitor/history ACK (tables 47/48/49).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809MonitorAck {
    pub vehicle_no: String,
    pub color: u8,
    pub subtype: u16,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub result: u8,
}

impl Jt809MonitorAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let allowed: &[u8] = match self.subtype {
            0x9207 => &[0x00, 0x01, 0x02, 0xFF],
            0x9208 => &[0x00, 0x01, 0xFF],
            0x9209 => &[0x00, 0x01, 0x02, 0x03, 0xFF],
            _ => return Err(Jt809Error::BusinessBody),
        };
        if !allowed.contains(&self.result) {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = Vec::with_capacity(7);
        data.extend_from_slice(&self.source_type.to_be_bytes());
        data.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        data.push(self.result);
        encode_control_body(&self.vehicle_no, self.color, self.subtype, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 28 {
            return Err(Jt809Error::BusinessBody);
        }
        let subtype = u16::from_be_bytes([body[22], body[23]]);
        let allowed: &[u8] = match subtype {
            0x9207 => &[0x00, 0x01, 0x02, 0xFF],
            0x9208 => &[0x00, 0x01, 0xFF],
            0x9209 => &[0x00, 0x01, 0x02, 0x03, 0xFF],
            _ => return Err(Jt809Error::BusinessBody),
        };
        let (vehicle_no, color, data) = decode_control_body(body, subtype)?;
        if data.len() != 7 || !allowed.contains(&data[6]) {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            subtype,
            source_type: u16::from_be_bytes([data[0], data[1]]),
            source_msg_sn: u32::from_be_bytes([data[2], data[3], data[4], data[5]]),
            result: data[6],
        })
    }
}

/// 0x9205 start location-exchange request (table 45).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809ReturnStartupRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub reason: u8,
}

impl Jt809ReturnStartupRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if !matches!(self.reason, 0x00 | 0x01 | 0x02 | 0xFF) {
            return Err(Jt809Error::BusinessBody);
        }
        encode_control_body(&self.vehicle_no, self.color, 0x9205, &[self.reason])
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9205)?;
        if data.len() != 1 || !matches!(data[0], 0x00 | 0x01 | 0x02 | 0xFF) {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            reason: data[0],
        })
    }
}

/// 0x9206 end location-exchange request (table 46), empty data part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809ReturnEndRequest {
    pub vehicle_no: String,
    pub color: u8,
}

impl Jt809ReturnEndRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, 0x9206, &[])
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9206)?;
        if !data.is_empty() {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self { vehicle_no, color })
    }
}

/// 0x1209 history-location replay request (table 33).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809HistoryRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub start_time: u64,
    pub end_time: u64,
}

impl Jt809HistoryRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.end_time < self.start_time {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&self.start_time.to_be_bytes());
        data.extend_from_slice(&self.end_time.to_be_bytes());
        encode_control_body(&self.vehicle_no, self.color, SUB_APPLY_HISTORY, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_APPLY_HISTORY)?;
        if data.len() != 16 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            start_time: u64::from_be_bytes(data[0..8].try_into().unwrap()),
            end_time: u64::from_be_bytes(data[8..16].try_into().unwrap()),
        })
    }
}

/// 0x1207 apply to monitor a specified vehicle (table 31).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809MonitorApply {
    pub vehicle_no: String,
    pub color: u8,
    pub start_time: u64,
    pub end_time: u64,
}

impl Jt809MonitorApply {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.end_time < self.start_time {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&self.start_time.to_be_bytes());
        data.extend_from_slice(&self.end_time.to_be_bytes());
        encode_control_body(&self.vehicle_no, self.color, SUB_APPLY_MONITOR, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_APPLY_MONITOR)?;
        if data.len() != 16 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            start_time: u64::from_be_bytes(data[0..8].try_into().unwrap()),
            end_time: u64::from_be_bytes(data[8..16].try_into().unwrap()),
        })
    }
}

/// 0x1208 cancel monitor request (table 32), empty data part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809MonitorCancel {
    pub vehicle_no: String,
    pub color: u8,
}

impl Jt809MonitorCancel {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, SUB_CANCEL_MONITOR, &[])
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_CANCEL_MONITOR)?;
        if !data.is_empty() {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self { vehicle_no, color })
    }
}

/// 0x1205/0x1206 start/end location-exchange ACK (tables 29/30).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809ReturnAck {
    pub vehicle_no: String,
    pub color: u8,
    pub subtype: u16,
    pub source_type: u16,
    pub source_msg_sn: u32,
}

impl Jt809ReturnAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.subtype != SUB_RETURN_STARTUP_ACK && self.subtype != SUB_RETURN_END_ACK {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = Vec::with_capacity(10);
        data.extend_from_slice(&self.source_type.to_be_bytes());
        data.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        encode_control_body(&self.vehicle_no, self.color, self.subtype, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 28 {
            return Err(Jt809Error::BusinessBody);
        }
        let subtype = u16::from_be_bytes([body[22], body[23]]);
        if subtype != SUB_RETURN_STARTUP_ACK && subtype != SUB_RETURN_END_ACK {
            return Err(Jt809Error::BusinessBody);
        }
        let (vehicle_no, color, data) = decode_control_body(body, subtype)?;
        if data.len() != 10 {
            return Err(Jt809Error::BusinessBody);
        }
        if u32::from_be_bytes([data[6], data[7], data[8], data[9]]) != 0 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            subtype,
            source_type: u16::from_be_bytes([data[0], data[1]]),
            source_msg_sn: u32::from_be_bytes([data[2], data[3], data[4], data[5]]),
        })
    }
}

/// 0x1201 vehicle registration (table 19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809VehicleRegister {
    pub vehicle_no: String,
    pub color: u8,
    pub platform_id: [u8; 11],
    pub producer_id: [u8; 11],
    pub terminal_model: [u8; 30],
    pub imei: String,
    pub terminal_id: [u8; 30],
    pub sim_code: String,
}

impl Jt809VehicleRegister {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut data = Vec::with_capacity(110);
        data.extend_from_slice(&self.platform_id);
        data.extend_from_slice(&self.producer_id);
        data.extend_from_slice(&self.terminal_model);
        data.extend_from_slice(&fixed_gbk(&self.imei, 15)?);
        data.extend_from_slice(&self.terminal_id);
        data.extend_from_slice(&fixed_gbk(&self.sim_code, 13)?);
        encode_control_body(&self.vehicle_no, self.color, SUB_REGISTER, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_REGISTER)?;
        if data.len() != 110 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            platform_id: data[0..11].try_into().unwrap(),
            producer_id: data[11..22].try_into().unwrap(),
            terminal_model: data[22..52].try_into().unwrap(),
            imei: read_gbk(&data[52..67])?,
            terminal_id: data[67..97].try_into().unwrap(),
            sim_code: read_gbk(&data[97..110])?,
        })
    }
}

/// 0x120E active route upload (table 38): 808 0x8606 route bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DrvLineUpload {
    pub vehicle_no: String,
    pub color: u8,
    pub route: Vec<u8>,
}

impl Jt809DrvLineUpload {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, SUB_DRVLINE, &self.route)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_DRVLINE)?;
        Ok(Self {
            vehicle_no,
            color,
            route: data.to_vec(),
        })
    }
}

/// 0x120B e-waybill ACK (table 35).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809EWaybillAck {
    pub vehicle_no: String,
    pub color: u8,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub bill: Vec<u8>,
}

impl Jt809EWaybillAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut data = Vec::with_capacity(10 + self.bill.len());
        data.extend_from_slice(&self.source_type.to_be_bytes());
        data.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        data.extend_from_slice(&(self.bill.len() as u32).to_be_bytes());
        data.extend_from_slice(&self.bill);
        encode_control_body(&self.vehicle_no, self.color, SUB_EWAYBILL_ACK, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_EWAYBILL_ACK)?;
        if data.len() < 10 {
            return Err(Jt809Error::BusinessBody);
        }
        let len = u32::from_be_bytes([data[6], data[7], data[8], data[9]]) as usize;
        if data.len() != 10 + len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            source_type: u16::from_be_bytes([data[0], data[1]]),
            source_msg_sn: u32::from_be_bytes([data[2], data[3], data[4], data[5]]),
            bill: data[10..].to_vec(),
        })
    }
}

/// 0x120D active e-waybill upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809EWaybillUpload {
    pub vehicle_no: String,
    pub color: u8,
    pub bill: Vec<u8>,
}

impl Jt809EWaybillUpload {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut data = Vec::with_capacity(4 + self.bill.len());
        data.extend_from_slice(&(self.bill.len() as u32).to_be_bytes());
        data.extend_from_slice(&self.bill);
        encode_control_body(&self.vehicle_no, self.color, SUB_EWAYBILL, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_EWAYBILL)?;
        if data.len() < 4 {
            return Err(Jt809Error::BusinessBody);
        }
        let len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() != 4 + len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            bill: data[4..].to_vec(),
        })
    }
}

/// 0x120A driver identity ACK (amended table 34).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DriverIdentityAck {
    pub vehicle_no: String,
    pub color: u8,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub identity: Jt809DriverIdentity,
}

impl Jt809DriverIdentityAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut data = Vec::with_capacity(311);
        data.extend_from_slice(&self.source_type.to_be_bytes());
        data.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        data.extend_from_slice(&self.identity.encode()?);
        encode_control_body(&self.vehicle_no, self.color, SUB_DRIVER_INFO_ACK, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_DRIVER_INFO_ACK)?;
        if data.len() != 311 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            source_type: u16::from_be_bytes([data[0], data[1]]),
            source_msg_sn: u32::from_be_bytes([data[2], data[3], data[4], data[5]]),
            identity: Jt809DriverIdentity::decode(&data[6..])?,
        })
    }
}

/// 0x120C active driver identity upload (amended table 36).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DriverIdentityUpload {
    pub vehicle_no: String,
    pub color: u8,
    pub identity: Jt809DriverIdentity,
}

impl Jt809DriverIdentityUpload {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(
            &self.vehicle_no,
            self.color,
            SUB_DRIVER_INFO,
            &self.identity.encode()?,
        )
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, SUB_DRIVER_INFO)?;
        Ok(Self {
            vehicle_no,
            color,
            identity: Jt809DriverIdentity::decode(data)?,
        })
    }
}

/// 8.3.8.1 vehicle location body used by photo ACK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809GnssBody {
    pub encrypt: u8,
    pub gnss: Vec<u8>,
    pub platform1: [u8; 11],
    pub alarm1: u32,
    pub platform2: [u8; 11],
    pub alarm2: u32,
    pub platform3: [u8; 11],
    pub alarm3: u32,
}

impl Jt809GnssBody {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.gnss.len() > u32::MAX as usize {
            return Err(Jt809Error::BusinessBody);
        }
        let mut out = Vec::with_capacity(50 + self.gnss.len());
        out.push(self.encrypt);
        out.extend_from_slice(&(self.gnss.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.gnss);
        out.extend_from_slice(&self.platform1);
        out.extend_from_slice(&self.alarm1.to_be_bytes());
        out.extend_from_slice(&self.platform2);
        out.extend_from_slice(&self.alarm2.to_be_bytes());
        out.extend_from_slice(&self.platform3);
        out.extend_from_slice(&self.alarm3.to_be_bytes());
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<(Self, usize), Jt809Error> {
        if body.len() < 50 {
            return Err(Jt809Error::BusinessBody);
        }
        let encrypt = body[0];
        let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
        let total = 5 + len + 45;
        if body.len() < total {
            return Err(Jt809Error::BusinessBody);
        }
        let at = 5 + len;
        Ok((
            Self {
                encrypt,
                gnss: body[5..at].to_vec(),
                platform1: body[at..at + 11].try_into().unwrap(),
                alarm1: u32::from_be_bytes(body[at + 11..at + 15].try_into().unwrap()),
                platform2: body[at + 15..at + 26].try_into().unwrap(),
                alarm2: u32::from_be_bytes(body[at + 26..at + 30].try_into().unwrap()),
                platform3: body[at + 30..at + 41].try_into().unwrap(),
                alarm3: u32::from_be_bytes(body[at + 41..at + 45].try_into().unwrap()),
            },
            total,
        ))
    }
}

/// 0x1502 vehicle photo ACK (table 75).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809PhotoAck {
    pub vehicle_no: String,
    pub color: u8,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub photo_flag: u8,
    pub gnss: Jt809GnssBody,
    pub lens_id: u8,
    pub size_type: u8,
    pub image_type: u8,
    pub photo: Vec<u8>,
}

impl Jt809PhotoAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let gnss = self.gnss.encode()?;
        let data_len = 7 + gnss.len() + 1 + 4 + 1 + 1 + self.photo.len();
        let mut data = Vec::with_capacity(data_len);
        data.extend_from_slice(&self.source_type.to_be_bytes());
        data.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        data.push(self.photo_flag);
        data.extend_from_slice(&gnss);
        data.push(self.lens_id);
        data.extend_from_slice(&(self.photo.len() as u32).to_be_bytes());
        data.push(self.size_type);
        data.push(self.image_type);
        data.extend_from_slice(&self.photo);
        encode_control_body(&self.vehicle_no, self.color, 0x1502, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x1502)?;
        if data.len() < 7 + 55 {
            return Err(Jt809Error::BusinessBody);
        }
        let source_type = u16::from_be_bytes([data[0], data[1]]);
        let source_msg_sn = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
        let photo_flag = data[6];
        let (gnss, gnss_len) = Jt809GnssBody::decode(&data[7..])?;
        let mut offset = 7 + gnss_len;
        if data.len() < offset + 7 {
            return Err(Jt809Error::BusinessBody);
        }
        let lens_id = data[offset];
        offset += 1;
        let photo_len = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let size_type = data[offset];
        let image_type = data[offset + 1];
        offset += 2;
        if data.len() != offset + photo_len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            source_type,
            source_msg_sn,
            photo_flag,
            gnss,
            lens_id,
            size_type,
            image_type,
            photo: data[offset..].to_vec(),
        })
    }
}

/// 0x1402 alarm report (JT/T 809-2019 table 64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809AlarmReport {
    pub platform_id: [u8; 11],
    pub warn_type: u16,
    pub warn_time: u64,
    pub start_time: u64,
    pub end_time: u64,
    pub vehicle_no: String,
    pub vehicle_color: u8,
    pub target_platform_id: [u8; 11],
    pub drvline_id: u32,
    pub info: Vec<u8>,
}

impl Jt809AlarmReport {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let (plate, _, bad) = encoding_rs::GBK.encode(&self.vehicle_no);
        if bad || plate.len() > 21 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = 78 + self.info.len();
        let mut out = Vec::with_capacity(6 + data_len);
        out.extend_from_slice(&0x1402u16.to_be_bytes());
        out.extend_from_slice(&(data_len as u32).to_be_bytes());
        out.extend_from_slice(&self.platform_id);
        out.extend_from_slice(&self.warn_type.to_be_bytes());
        out.extend_from_slice(&self.warn_time.to_be_bytes());
        out.extend_from_slice(&self.start_time.to_be_bytes());
        out.extend_from_slice(&self.end_time.to_be_bytes());
        let mut fixed = [0u8; 21];
        fixed[..plate.len()].copy_from_slice(&plate);
        out.extend_from_slice(&fixed);
        out.push(self.vehicle_color);
        out.extend_from_slice(&self.target_platform_id);
        out.extend_from_slice(&self.drvline_id.to_be_bytes());
        out.extend_from_slice(&(self.info.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.info);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 6 + 78 || u16::from_be_bytes([body[0], body[1]]) != 0x1402 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = u32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        if body.len() != 6 + data_len {
            return Err(Jt809Error::BusinessBody);
        }
        let data = &body[6..];
        let info_len = u32::from_be_bytes([data[74], data[75], data[76], data[77]]) as usize;
        if data.len() != 78 + info_len {
            return Err(Jt809Error::BusinessBody);
        }
        let end = data[37..58].iter().position(|b| *b == 0).unwrap_or(21);
        let (plate, _, bad) = encoding_rs::GBK.decode(&data[37..37 + end]);
        if bad {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            platform_id: data[0..11].try_into().unwrap(),
            warn_type: u16::from_be_bytes([data[11], data[12]]),
            warn_time: u64::from_be_bytes(data[13..21].try_into().unwrap()),
            start_time: u64::from_be_bytes(data[21..29].try_into().unwrap()),
            end_time: u64::from_be_bytes(data[29..37].try_into().unwrap()),
            vehicle_no: plate.into_owned(),
            vehicle_color: data[58],
            target_platform_id: data[59..70].try_into().unwrap(),
            drvline_id: u32::from_be_bytes(data[70..74].try_into().unwrap()),
            info: data[78..].to_vec(),
        })
    }
}

/// 0x1412 active alarm-result report (table 65).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809AlarmResultAck {
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub result: u8,
}

impl Jt809AlarmResultAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.result > 3 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut out = Vec::with_capacity(13);
        out.extend_from_slice(&0x1412u16.to_be_bytes());
        out.extend_from_slice(&7u32.to_be_bytes());
        out.extend_from_slice(&self.source_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out.push(self.result);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 13
            || u16::from_be_bytes([body[0], body[1]]) != 0x1412
            || u32::from_be_bytes([body[2], body[3], body[4], body[5]]) != 7
            || body[12] > 3
        {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            source_type: u16::from_be_bytes([body[6], body[7]]),
            source_msg_sn: u32::from_be_bytes([body[8], body[9], body[10], body[11]]),
            result: body[12],
        })
    }
}

/// Parse the table-91 vehicle static info string: `KEY:=VALUE;KEY:=VALUE`.
pub fn parse_vehicle_static_info(
    bytes: &[u8],
) -> Result<std::collections::BTreeMap<String, String>, Jt809Error> {
    let (text, _, bad) = encoding_rs::GBK.decode(bytes);
    if bad {
        return Err(Jt809Error::BusinessBody);
    }
    let mut out = std::collections::BTreeMap::new();
    for item in text.split(';') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (key, value) = item.split_once(":=").ok_or(Jt809Error::BusinessBody)?;
        let key = key.trim();
        if key.is_empty() {
            return Err(Jt809Error::BusinessBody);
        }
        out.insert(key.to_string(), value.trim().to_string());
    }
    Ok(out)
}

/// 0x1601 vehicle static information ACK (table 87).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809VehicleStaticAck {
    pub vehicle_no: String,
    pub color: u8,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub car_info: Vec<u8>,
}

impl Jt809VehicleStaticAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut data = Vec::with_capacity(6 + self.car_info.len());
        data.extend_from_slice(&self.source_type.to_be_bytes());
        data.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        data.extend_from_slice(&self.car_info);
        encode_control_body(&self.vehicle_no, self.color, 0x1601, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x1601)?;
        if data.len() < 6 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            source_type: u16::from_be_bytes([data[0], data[1]]),
            source_msg_sn: u32::from_be_bytes([data[2], data[3], data[4], data[5]]),
            car_info: data[6..].to_vec(),
        })
    }
}

/// 0x1602 vehicle route supplement ACK (table 86).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809DrvLineAck {
    pub vehicle_no: String,
    pub color: u8,
    pub route: Vec<u8>,
}

impl Jt809DrvLineAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(&self.vehicle_no, self.color, 0x1602, &self.route)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x1602)?;
        Ok(Self {
            vehicle_no,
            color,
            route: data.to_vec(),
        })
    }
}

/// 0x9501 vehicle one-way listening request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809MonitorRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub monitor_tel: String,
}

impl Jt809MonitorRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(
            &self.vehicle_no,
            self.color,
            0x9501,
            &fixed_ascii_pad(&self.monitor_tel, 20)?,
        )
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9501)?;
        if data.len() != 20 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            monitor_tel: trim_ascii_fixed(data)?,
        })
    }
}

/// 0x9502 vehicle photo request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809PhotoRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub lens_id: u8,
    pub size: u8,
}

impl Jt809PhotoRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        encode_control_body(
            &self.vehicle_no,
            self.color,
            0x9502,
            &[self.lens_id, self.size],
        )
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9502)?;
        if data.len() != 2 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            lens_id: data[0],
            size: data[1],
        })
    }
}

/// 0x9503 vehicle text-message request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809TextRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub priority: u8,
    pub message: Vec<u8>,
}

impl Jt809TextRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.message.len() > 1024 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut data = Vec::with_capacity(5 + self.message.len());
        data.push(self.priority);
        data.extend_from_slice(&(self.message.len() as u32).to_be_bytes());
        data.extend_from_slice(&self.message);
        encode_control_body(&self.vehicle_no, self.color, 0x9503, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9503)?;
        if data.len() < 5 {
            return Err(Jt809Error::BusinessBody);
        }
        let len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;
        if len > 1024 || data.len() != 5 + len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            priority: data[0],
            message: data[5..].to_vec(),
        })
    }
}

/// 0x9504 travel-record request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809TravelRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub start_time: u64,
    pub end_time: u64,
    pub command_type: u8,
}

impl Jt809TravelRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut data = Vec::with_capacity(17);
        data.extend_from_slice(&self.start_time.to_be_bytes());
        data.extend_from_slice(&self.end_time.to_be_bytes());
        data.push(self.command_type);
        encode_control_body(&self.vehicle_no, self.color, 0x9504, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9504)?;
        if data.len() != 17 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            start_time: u64::from_be_bytes(data[..8].try_into().unwrap()),
            end_time: u64::from_be_bytes(data[8..16].try_into().unwrap()),
            command_type: data[16],
        })
    }
}

/// 0x9505 emergency monitoring access request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809EmergencyRequest {
    pub vehicle_no: String,
    pub color: u8,
    pub auth_code: [u8; 10],
    pub apn: String,
    pub username: String,
    pub password: String,
    pub server_ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub end_time: u64,
}

impl Jt809EmergencyRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let mut data = Vec::with_capacity(145);
        data.extend_from_slice(&self.auth_code);
        data.extend_from_slice(&fixed_ascii_pad(&self.apn, 20)?);
        data.extend_from_slice(&fixed_ascii_pad(&self.username, 49)?);
        data.extend_from_slice(&fixed_ascii_pad(&self.password, 22)?);
        data.extend_from_slice(&fixed_ascii_pad(&self.server_ip, 32)?);
        data.extend_from_slice(&self.tcp_port.to_be_bytes());
        data.extend_from_slice(&self.udp_port.to_be_bytes());
        data.extend_from_slice(&self.end_time.to_be_bytes());
        encode_control_body(&self.vehicle_no, self.color, 0x9505, &data)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let (vehicle_no, color, data) = decode_control_body(body, 0x9505)?;
        if data.len() != 145 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no,
            color,
            auth_code: data[..10].try_into().unwrap(),
            apn: trim_ascii_fixed(&data[10..30])?,
            username: trim_ascii_fixed(&data[30..79])?,
            password: trim_ascii_fixed(&data[79..101])?,
            server_ip: trim_ascii_fixed(&data[101..133])?,
            tcp_port: u16::from_be_bytes([data[133], data[134]]),
            udp_port: u16::from_be_bytes([data[135], data[136]]),
            end_time: u64::from_be_bytes(data[137..145].try_into().unwrap()),
        })
    }
}

/// 0x1504 vehicle travel-record ACK (JT/T 809-2019 table 77).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809TravelRecordAck {
    pub vehicle_no: String,
    pub color: u8,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub command_type: u8,
    pub travel_data: Vec<u8>,
}

impl Jt809TravelRecordAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        let (plate, _, bad) = encoding_rs::GBK.encode(&self.vehicle_no);
        if bad || plate.len() > 21 || self.travel_data.len() > u32::MAX as usize {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = 2 + 4 + 1 + 4 + self.travel_data.len();
        let mut out = Vec::with_capacity(21 + 1 + 2 + 4 + data_len);
        let mut fixed = [0u8; 21];
        fixed[..plate.len()].copy_from_slice(&plate);
        out.extend_from_slice(&fixed);
        out.push(self.color);
        out.extend_from_slice(&0x1504u16.to_be_bytes());
        out.extend_from_slice(&(data_len as u32).to_be_bytes());
        out.extend_from_slice(&self.source_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out.push(self.command_type);
        out.extend_from_slice(&(self.travel_data.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.travel_data);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 39 {
            return Err(Jt809Error::BusinessBody);
        }
        let subtype = u16::from_be_bytes([body[22], body[23]]);
        if subtype != 0x1504 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = u32::from_be_bytes([body[24], body[25], body[26], body[27]]) as usize;
        if body.len() != 28 + data_len {
            return Err(Jt809Error::BusinessBody);
        }
        let travel_len = u32::from_be_bytes([body[35], body[36], body[37], body[38]]) as usize;
        if data_len != 11 + travel_len || body.len() != 39 + travel_len {
            return Err(Jt809Error::BusinessBody);
        }
        let end = body[..21].iter().position(|b| *b == 0).unwrap_or(21);
        let (vehicle_no, _, bad) = encoding_rs::GBK.decode(&body[..end]);
        if bad {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no: vehicle_no.into_owned(),
            color: body[21],
            source_type: u16::from_be_bytes([body[28], body[29]]),
            source_msg_sn: u32::from_be_bytes([body[30], body[31], body[32], body[33]]),
            command_type: body[34],
            travel_data: body[39..].to_vec(),
        })
    }
}

/// Common 0x1501/0x1503/0x1505 vehicle-control ACK body.
/// VEHICLE_NO is 21 bytes; DATA_LENGTH counts the trailing source/result data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809VehicleControlAck {
    pub vehicle_no: String,
    pub color: u8,
    pub subtype: u16,
    pub source_type: u16,
    pub source_msg_sn: u32,
    pub result: u8,
}

impl Jt809VehicleControlAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if !matches!(self.subtype, 0x1501 | 0x1503 | 0x1505) {
            return Err(Jt809Error::BusinessBody);
        }
        let (plate_bytes, _, bad) = encoding_rs::GBK.encode(&self.vehicle_no);
        if bad || plate_bytes.len() > 21 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut out = Vec::with_capacity(35);
        let mut plate = [0u8; 21];
        plate[..plate_bytes.len()].copy_from_slice(&plate_bytes);
        out.extend_from_slice(&plate);
        out.push(self.color);
        out.extend_from_slice(&self.subtype.to_be_bytes());
        out.extend_from_slice(&7u32.to_be_bytes());
        out.extend_from_slice(&self.source_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out.push(self.result);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 35 {
            return Err(Jt809Error::BusinessBody);
        }
        let subtype = u16::from_be_bytes([body[22], body[23]]);
        if !matches!(subtype, 0x1501 | 0x1503 | 0x1505) {
            return Err(Jt809Error::BusinessBody);
        }
        if u32::from_be_bytes([body[24], body[25], body[26], body[27]]) != 7 {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            vehicle_no: {
                let end = body[..21].iter().position(|b| *b == 0).unwrap_or(21);
                let (value, _, bad) = encoding_rs::GBK.decode(&body[..end]);
                if bad {
                    return Err(Jt809Error::BusinessBody);
                }
                value.into_owned()
            },
            color: body[21],
            subtype,
            source_type: u16::from_be_bytes([body[28], body[29]]),
            source_msg_sn: u32::from_be_bytes([body[30], body[31], body[32], body[33]]),
            result: body[34],
        })
    }
}

/// 0x9301 platform check-in request (JT/T 809-2019 8.3.4.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809PlatformQueryRequest {
    pub object_type: u8,
    pub object_id: String,
    pub answer_minutes: u8,
    pub info_id: u32,
    pub info: Vec<u8>,
}

impl Jt809PlatformQueryRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.info.len() > u32::MAX as usize {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = 1 + 20 + 1 + 4 + 4 + self.info.len();
        let mut out = Vec::with_capacity(6 + data_len);
        out.extend_from_slice(&0x9301u16.to_be_bytes());
        out.extend_from_slice(&(data_len as u32).to_be_bytes());
        out.push(self.object_type);
        out.extend_from_slice(&fixed_ascii_n(&self.object_id, 20)?);
        out.push(self.answer_minutes);
        out.extend_from_slice(&self.info_id.to_be_bytes());
        out.extend_from_slice(&(self.info.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.info);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 6 + 30 {
            return Err(Jt809Error::BusinessBody);
        }
        if u16::from_be_bytes([body[0], body[1]]) != 0x9301 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = u32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        if body.len() != 6 + data_len {
            return Err(Jt809Error::BusinessBody);
        }
        let data = &body[6..];
        let object_type = data[0];
        let object_id = trim_ascii_fixed(&data[1..21])?;
        let answer_minutes = data[21];
        let info_id = u32::from_be_bytes([data[22], data[23], data[24], data[25]]);
        let info_len = u32::from_be_bytes([data[26], data[27], data[28], data[29]]) as usize;
        if data.len() != 30 + info_len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            object_type,
            object_id,
            answer_minutes,
            info_id,
            info: data[30..].to_vec(),
        })
    }
}

/// JT/T 809 body cipher (8.2.7). M1/IA1/IC1 are platform-negotiated, so
/// they are configuration rather than hard-coded constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809Cipher {
    pub m1: u32,
    pub ia1: u32,
    pub ic1: u32,
}

impl Jt809Cipher {
    pub fn new(m1: u32, ia1: u32, ic1: u32) -> Self {
        Self { m1, ia1, ic1 }
    }

    /// Encrypt/decrypt in place. The operation is its own inverse.
    pub fn apply(&self, key: u32, buffer: &mut [u8]) {
        let mut key = if key == 0 { 1 } else { key };
        let mkey = if self.m1 == 0 { 1 } else { self.m1 };
        for byte in buffer {
            key = self.ia1.wrapping_mul(key % mkey).wrapping_add(self.ic1);
            *byte ^= ((key >> 20) & 0xFF) as u8;
        }
    }
}

/// Per-subtype message sequence tracking and gap detection (JT/T 809 7).
#[derive(Debug, Default)]
pub struct Jt809SequenceTracker {
    next: BTreeMap<u16, u32>,
}

impl Jt809SequenceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_sn(&mut self, data_type: u16) -> u32 {
        let entry = self.next.entry(data_type).or_insert(0);
        *entry
    }

    /// Observe an incoming sequence number. Returns missing ranges
    /// `(start, end)` when a gap is detected.
    pub fn observe(&mut self, data_type: u16, sn: u32) -> Vec<(u32, u32)> {
        let expected = *self.next.get(&data_type).unwrap_or(&0);
        if sn == expected {
            self.next.insert(data_type, sn.wrapping_add(1));
            Vec::new()
        } else if sn > expected {
            self.next.insert(data_type, sn.wrapping_add(1));
            vec![(expected, sn - 1)]
        } else {
            Vec::new()
        }
    }

    /// Record a received retransmission without moving the expected cursor
    /// backwards.
    pub fn observe_retransmit(&mut self, data_type: u16, sn: u32) {
        let expected = *self.next.get(&data_type).unwrap_or(&0);
        if sn == expected {
            self.next.insert(data_type, sn.wrapping_add(1));
        }
    }
}

/// 0x1303/0x9303 platform-message retransmit request (8.3.4.1.4).
/// The serial list is interpreted as (start_sn, end_sn) pairs; a single
/// message uses the same start and end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809RetransmitRequest {
    pub data_type: u16,
    pub link_data_type: u16,
    pub ranges: Vec<(u32, u32)>,
    pub time: u64,
}

impl Jt809RetransmitRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.ranges.len() > u8::MAX as usize {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = 2 + 1 + self.ranges.len() * 8 + 8;
        let mut out = Vec::with_capacity(6 + data_len);
        out.extend_from_slice(&self.data_type.to_be_bytes());
        out.extend_from_slice(&(data_len as u32).to_be_bytes());
        out.extend_from_slice(&self.link_data_type.to_be_bytes());
        out.push(self.ranges.len() as u8);
        for (start, end) in &self.ranges {
            out.extend_from_slice(&start.to_be_bytes());
            out.extend_from_slice(&end.to_be_bytes());
        }
        out.extend_from_slice(&self.time.to_be_bytes());
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 6 + 11 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_type = u16::from_be_bytes([body[0], body[1]]);
        if data_type != 0x1303 && data_type != 0x9303 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = u32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        if body.len() != 6 + data_len {
            return Err(Jt809Error::BusinessBody);
        }
        let data = &body[6..];
        let link_data_type = u16::from_be_bytes([data[0], data[1]]);
        let count = data[2] as usize;
        if data.len() != 3 + count * 8 + 8 {
            return Err(Jt809Error::BusinessBody);
        }
        let mut offset = 3;
        let mut ranges = Vec::with_capacity(count);
        for _ in 0..count {
            let start = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
            let end = u32::from_be_bytes(data[offset + 4..offset + 8].try_into().unwrap());
            ranges.push((start, end));
            offset += 8;
        }
        Ok(Self {
            data_type,
            link_data_type,
            ranges,
            time: u64::from_be_bytes(data[offset..offset + 8].try_into().unwrap()),
        })
    }
}

/// 0x1301 platform check-in ACK (JT/T 809-2019 8.3.4.1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809PlatformQueryAck {
    pub object_type: u8,
    pub responder: String,
    pub responder_tel: String,
    pub object_id: String,
    pub source_data_type: u16,
    pub source_msg_sn: u32,
    pub info: Vec<u8>,
}

impl Jt809PlatformQueryAck {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.info.len() > u32::MAX as usize {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = 1 + 16 + 20 + 20 + 2 + 4 + 4 + self.info.len();
        let mut out = Vec::with_capacity(6 + data_len);
        out.extend_from_slice(&0x1301u16.to_be_bytes());
        out.extend_from_slice(&(data_len as u32).to_be_bytes());
        out.push(self.object_type);
        out.extend_from_slice(&fixed_ascii_n(&self.responder, 16)?);
        out.extend_from_slice(&fixed_ascii_n(&self.responder_tel, 20)?);
        out.extend_from_slice(&fixed_ascii_n(&self.object_id, 20)?);
        out.extend_from_slice(&self.source_data_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out.extend_from_slice(&(self.info.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.info);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 6 + 67 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_type = u16::from_be_bytes([body[0], body[1]]);
        if data_type != 0x1301 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_len = u32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        if body.len() != 6 + data_len {
            return Err(Jt809Error::BusinessBody);
        }
        let data = &body[6..];
        let object_type = data[0];
        let responder = trim_ascii_fixed(&data[1..17])?;
        let responder_tel = trim_ascii_fixed(&data[17..37])?;
        let object_id = trim_ascii_fixed(&data[37..57])?;
        let source_data_type = u16::from_be_bytes([data[57], data[58]]);
        let source_msg_sn = u32::from_be_bytes([data[59], data[60], data[61], data[62]]);
        let info_len = u32::from_be_bytes([data[63], data[64], data[65], data[66]]) as usize;
        if data.len() != 67 + info_len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            object_type,
            responder,
            responder_tel,
            object_id,
            source_data_type,
            source_msg_sn,
            info: data[67..].to_vec(),
        })
    }
}

fn fixed_ascii_n(value: &str, size: usize) -> Result<Vec<u8>, Jt809Error> {
    if !value.is_ascii() || value.len() > size {
        return Err(Jt809Error::BusinessBody);
    }
    let mut out = vec![0u8; size];
    out[..value.len()].copy_from_slice(value.as_bytes());
    Ok(out)
}

fn trim_ascii_fixed(bytes: &[u8]) -> Result<String, Jt809Error> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_string)
        .map_err(|_| Jt809Error::BusinessBody)
}

/// 0x1300/0x9300 platform-message envelope (JT/T 809-2019 8.3.4.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809PlatformMessage {
    pub data_type: u16,
    pub data: Vec<u8>,
}

impl Jt809PlatformMessage {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if self.data.len() > u32::MAX as usize {
            return Err(Jt809Error::BusinessBody);
        }
        let mut out = Vec::with_capacity(6 + self.data.len());
        out.extend_from_slice(&self.data_type.to_be_bytes());
        out.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.data);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 6 {
            return Err(Jt809Error::BusinessBody);
        }
        let data_type = u16::from_be_bytes([body[0], body[1]]);
        let len = u32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        if body.len() != 6 + len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            data_type,
            data: body[6..].to_vec(),
        })
    }
}

/// 0x1302 platform-message ACK (8.3.4.1.3): source type + source serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809InfoAck {
    pub source_data_type: u16,
    pub source_msg_sn: u32,
}

impl Jt809InfoAck {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12);
        out.extend_from_slice(&0x1302u16.to_be_bytes());
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&self.source_data_type.to_be_bytes());
        out.extend_from_slice(&self.source_msg_sn.to_be_bytes());
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 12
            || u16::from_be_bytes([body[0], body[1]]) != 0x1302
            || u32::from_be_bytes([body[2], body[3], body[4], body[5]]) != 6
        {
            return Err(Jt809Error::AckBody);
        }
        Ok(Self {
            source_data_type: u16::from_be_bytes([body[6], body[7]]),
            source_msg_sn: u32::from_be_bytes([body[8], body[9], body[10], body[11]]),
        })
    }
}

/// 0x9001 platform acknowledgement candidate: ack serial u32 + result u8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt809Ack {
    pub ack_sn: u32,
    pub result: u8,
}

impl Jt809Ack {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5);
        out.extend_from_slice(&self.ack_sn.to_be_bytes());
        out.push(self.result);
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() != 5 {
            return Err(Jt809Error::AckBody);
        }
        Ok(Self {
            ack_sn: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
            result: body[4],
        })
    }
}

/// 0x1203/0x1202 alarm payload candidate: alarm type u32, lat/lon i32 /1e7,
/// BCD[6] time, opaque tail.
#[derive(Debug, Clone, PartialEq)]
pub struct Jt809AlarmData {
    pub alarm_type: u32,
    pub lat: f64,
    pub lon: f64,
    pub ts: [u8; 6],
    pub tail: Vec<u8>,
}

impl Jt809AlarmData {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if !(-90.0..=90.0).contains(&self.lat) || !(-180.0..=180.0).contains(&self.lon) {
            return Err(Jt809Error::AlarmBody);
        }
        let mut out = Vec::with_capacity(18 + self.tail.len());
        out.extend_from_slice(&self.alarm_type.to_be_bytes());
        out.extend_from_slice(&((self.lat * 10_000_000.0) as i32).to_be_bytes());
        out.extend_from_slice(&((self.lon * 10_000_000.0) as i32).to_be_bytes());
        out.extend_from_slice(&self.ts);
        out.extend_from_slice(&self.tail);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 18 {
            return Err(Jt809Error::AlarmBody);
        }
        let lat = i32::from_be_bytes([body[4], body[5], body[6], body[7]]) as f64 / 10_000_000.0;
        let lon = i32::from_be_bytes([body[8], body[9], body[10], body[11]]) as f64 / 10_000_000.0;
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Err(Jt809Error::AlarmBody);
        }
        Ok(Self {
            alarm_type: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
            lat,
            lon,
            ts: body[12..18].try_into().map_err(|_| Jt809Error::AlarmBody)?,
            tail: body[18..].to_vec(),
        })
    }
}

/// 0x1200 business envelope: sub_type u16 + data_len u32 + data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809BusinessEnvelope {
    pub sub_type: u16,
    pub data: Vec<u8>,
}

impl Jt809BusinessEnvelope {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(6 + self.data.len());
        out.extend_from_slice(&self.sub_type.to_be_bytes());
        out.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 6 {
            return Err(Jt809Error::BusinessBody);
        }
        let sub_type = u16::from_be_bytes([body[0], body[1]]);
        let len = u32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        if body.len() != 6 + len {
            return Err(Jt809Error::BusinessBody);
        }
        Ok(Self {
            sub_type,
            data: body[6..].to_vec(),
        })
    }
}

/// 0x1201 real-time location block (candidate layout): lat i32 /1e7,
/// lon i32 /1e7, altitude u16, speed u16 /10, heading u16, BCD[6], tail.
#[derive(Debug, Clone, PartialEq)]
pub struct Jt809LocationData {
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: u16,
    pub speed_kmh: f64,
    pub heading: u16,
    pub ts: [u8; 6],
    pub tail: Vec<u8>,
}

impl Jt809LocationData {
    pub fn encode(&self) -> Result<Vec<u8>, Jt809Error> {
        if !(-90.0..=90.0).contains(&self.lat)
            || !(-180.0..=180.0).contains(&self.lon)
            || self.heading > 359
        {
            return Err(Jt809Error::LocationBody);
        }
        let mut out = Vec::with_capacity(20 + self.tail.len());
        out.extend_from_slice(&((self.lat * 10_000_000.0) as i32).to_be_bytes());
        out.extend_from_slice(&((self.lon * 10_000_000.0) as i32).to_be_bytes());
        out.extend_from_slice(&self.altitude_m.to_be_bytes());
        out.extend_from_slice(&((self.speed_kmh.max(0.0) * 10.0) as u16).to_be_bytes());
        out.extend_from_slice(&self.heading.to_be_bytes());
        out.extend_from_slice(&self.ts);
        out.extend_from_slice(&self.tail);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        if body.len() < 20 {
            return Err(Jt809Error::LocationBody);
        }
        let lat = i32::from_be_bytes([body[0], body[1], body[2], body[3]]) as f64 / 10_000_000.0;
        let lon = i32::from_be_bytes([body[4], body[5], body[6], body[7]]) as f64 / 10_000_000.0;
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Err(Jt809Error::LocationBody);
        }
        let heading = u16::from_be_bytes([body[12], body[13]]);
        if heading > 359 {
            return Err(Jt809Error::LocationBody);
        }
        Ok(Self {
            lat,
            lon,
            altitude_m: u16::from_be_bytes([body[8], body[9]]),
            speed_kmh: u16::from_be_bytes([body[10], body[11]]) as f64 / 10.0,
            heading,
            ts: body[14..20]
                .try_into()
                .map_err(|_| Jt809Error::LocationBody)?,
            tail: body[20..].to_vec(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Jt809Inbound {
    LoginResponse(Jt809LoginResponse),
    Ack(Jt809Ack),
    Business(Jt809BusinessEnvelope),
    Unknown { msg_id: u16 },
}

/// Classify a decoded 809 frame into the subset this build understands.
pub fn classify(frame: &Jt809Frame) -> Result<Jt809Inbound, Jt809Error> {
    match frame.msg_id {
        MSG_LOGIN_RESPONSE => Ok(Jt809Inbound::LoginResponse(Jt809LoginResponse::decode(
            &frame.body,
        )?)),
        MSG_PLATFORM_ACK => Ok(Jt809Inbound::Ack(Jt809Ack::decode(&frame.body)?)),
        MSG_UP_DYNAMIC => Ok(Jt809Inbound::Business(Jt809BusinessEnvelope::decode(
            &frame.body,
        )?)),
        msg_id => Ok(Jt809Inbound::Unknown { msg_id }),
    }
}

impl Jt809Connection<TcpStream> {
    pub async fn connect_tcp(addr: &str, max_pending: usize) -> Result<Self, Jt809Error> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|error| Jt809Error::Io(error.to_string()))?;
        Ok(Self::new(stream, max_pending))
    }

    /// Connect, send 0x1001, and accept only a successful 0x1002 response.
    pub async fn connect_and_login(
        addr: &str,
        login: &Jt809Login,
        max_pending: usize,
    ) -> Result<Self, Jt809Error> {
        let mut connection = Self::connect_tcp(addr, max_pending).await?;
        connection.send_login(login).await?;
        match connection.receive().await? {
            Jt809Inbound::LoginResponse(response) if response.result == 0 => Ok(connection),
            Jt809Inbound::LoginResponse(response) => {
                Err(Jt809Error::LoginRejected(response.result))
            }
            _ => Err(Jt809Error::LoginResponse),
        }
    }
}

/// Exponential reconnect backoff with a hard attempt limit.
#[derive(Debug, Clone)]
pub struct Jt809Backoff {
    base: Duration,
    max: Duration,
    max_attempts: u32,
    attempts: u32,
}

impl Jt809Backoff {
    pub fn new(base: Duration, max: Duration, max_attempts: u32) -> Self {
        Self {
            base,
            max,
            max_attempts,
            attempts: 0,
        }
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Returns the next delay, or None after the attempt budget is exhausted.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.attempts >= self.max_attempts {
            return None;
        }
        let factor = 1u32.checked_shl(self.attempts).unwrap_or(u32::MAX);
        let delay = self.base.saturating_mul(factor).min(self.max);
        self.attempts += 1;
        Some(delay)
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
    }
}

/// Bounded retransmission window for 809 request/response correlation.
#[derive(Debug, Default)]
pub struct Jt809RetryWindow {
    sent: BTreeMap<u32, Instant>,
    max_pending: usize,
}

impl Jt809RetryWindow {
    pub fn new(max_pending: usize) -> Self {
        Self {
            sent: BTreeMap::new(),
            max_pending,
        }
    }

    /// Register an outgoing serial. Returns false when the window is full.
    pub fn mark_sent(&mut self, sn: u32) -> bool {
        if !self.sent.contains_key(&sn) && self.sent.len() >= self.max_pending {
            return false;
        }
        self.sent.insert(sn, Instant::now());
        true
    }

    /// Remove a serial after a successful acknowledgement.
    pub fn acknowledge(&mut self, sn: u32) -> bool {
        self.sent.remove(&sn).is_some()
    }

    /// Serials older than the threshold, without removing them.
    pub fn timed_out(&self, threshold: Duration) -> Vec<u32> {
        let now = Instant::now();
        self.sent
            .iter()
            .filter(|(_, sent)| now.duration_since(**sent) >= threshold)
            .map(|(sn, _)| *sn)
            .collect()
    }

    pub fn pending_len(&self) -> usize {
        self.sent.len()
    }

    /// Clear the window on reconnect; pending requests are not silently resent.
    pub fn reset(&mut self) {
        self.sent.clear();
    }
}

/// Connection lifecycle combining the handshake state and retry window.
#[derive(Debug, Default)]
pub struct Jt809Link {
    session: Jt809Session,
    retry: Jt809RetryWindow,
}

impl Jt809Link {
    pub fn new(max_pending: usize) -> Self {
        Self {
            session: Jt809Session::new(),
            retry: Jt809RetryWindow::new(max_pending),
        }
    }

    pub fn begin_login(&mut self, sn: u32) -> bool {
        if self.session.mark_login_sent().is_err() {
            return false;
        }
        self.retry.mark_sent(sn)
    }

    pub fn on_login_response(&mut self, sn: u32, result: u8) -> bool {
        self.retry.acknowledge(sn);
        self.session.on_login_response(result).is_ok()
    }

    pub fn acknowledge(&mut self, sn: u32) -> bool {
        self.retry.acknowledge(sn)
    }

    pub fn retry_mark(&mut self, sn: u32) -> bool {
        self.retry.mark_sent(sn)
    }

    pub fn timed_out(&self, threshold: Duration) -> Vec<u32> {
        self.retry.timed_out(threshold)
    }

    pub fn state(&self) -> Jt809SessionState {
        self.session.state()
    }

    pub fn pending_len(&self) -> usize {
        self.retry.pending_len()
    }

    /// Reconnect starts a fresh handshake and clears pending requests.
    pub fn reconnect(&mut self) {
        self.session = Jt809Session::new();
        self.retry.reset();
    }
}

/// Minimal async 809 connection loop over any byte stream.
impl Jt809Connection<tokio_rustls::client::TlsStream<TcpStream>> {
    /// Connect with server-certificate verification against `ca_pem`.
    pub async fn connect_tls(
        addr: &str,
        server_name: &str,
        ca_pem: &[u8],
        max_pending: usize,
    ) -> Result<Self, Jt809Error> {
        let mut roots = RootCertStore::empty();
        let mut reader = BufReader::new(ca_pem);
        for cert in rustls_pemfile::certs(&mut reader) {
            let cert = cert.map_err(|error| Jt809Error::Io(error.to_string()))?;
            roots
                .add(cert)
                .map_err(|error| Jt809Error::Io(error.to_string()))?;
        }
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));
        let server_name = ServerName::try_from(server_name.to_string())
            .map_err(|error| Jt809Error::Io(error.to_string()))?;
        let tcp = TcpStream::connect(addr)
            .await
            .map_err(|error| Jt809Error::Io(error.to_string()))?;
        let stream = connector
            .connect(server_name, tcp)
            .await
            .map_err(|error| Jt809Error::Io(error.to_string()))?;
        Ok(Self::new(stream, max_pending))
    }
}

#[derive(Debug)]
pub struct Jt809Connection<S> {
    stream: S,
    buffer: Vec<u8>,
    link: Jt809Link,
    next_sn: u32,
}

impl<S> Jt809Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: S, max_pending: usize) -> Self {
        Self {
            stream,
            buffer: Vec::with_capacity(1024),
            link: Jt809Link::new(max_pending),
            next_sn: 1,
        }
    }

    pub fn link(&self) -> &Jt809Link {
        &self.link
    }

    pub fn link_mut(&mut self) -> &mut Jt809Link {
        &mut self.link
    }

    pub async fn send_frame(&mut self, msg_id: u16, body: Vec<u8>) -> Result<u32, Jt809Error> {
        let sn = self.next_sn();
        if !self.link.retry_mark(sn) {
            return Err(Jt809Error::State);
        }
        let frame = Jt809Frame {
            msg_id,
            msg_sn: sn,
            encrypt: 0,
            compress: 0,
            body,
        };
        self.stream
            .write_all(&frame.encode())
            .await
            .map_err(|error| Jt809Error::Io(error.to_string()))?;
        Ok(sn)
    }

    fn next_sn(&mut self) -> u32 {
        let sn = self.next_sn;
        self.next_sn = self.next_sn.wrapping_add(1);
        sn
    }

    pub async fn send_login(&mut self, login: &Jt809Login) -> Result<u32, Jt809Error> {
        let sn = self.next_sn();
        if !self.link.begin_login(sn) {
            return Err(Jt809Error::State);
        }
        let frame = Jt809Frame {
            msg_id: MSG_LOGIN_REQUEST,
            msg_sn: sn,
            encrypt: 0,
            compress: 0,
            body: login.encode()?,
        };
        self.stream
            .write_all(&frame.encode())
            .await
            .map_err(|error| Jt809Error::Io(error.to_string()))?;
        Ok(sn)
    }

    pub async fn read_frame(&mut self) -> Result<Jt809Frame, Jt809Error> {
        loop {
            if let Some(start) = self.buffer.iter().position(|byte| *byte == JT809_START) {
                if start > 0 {
                    self.buffer.drain(..start);
                }
                if let Some(end) = self
                    .buffer
                    .iter()
                    .skip(1)
                    .position(|byte| *byte == JT809_END)
                {
                    let end = end + 1;
                    let raw = self.buffer[..=end].to_vec();
                    self.buffer.drain(..=end);
                    return Jt809Frame::decode(&raw);
                }
            } else {
                self.buffer.clear();
            }
            let mut chunk = [0u8; 2048];
            let read = self
                .stream
                .read(&mut chunk)
                .await
                .map_err(|error| Jt809Error::Io(error.to_string()))?;
            if read == 0 {
                return Err(Jt809Error::Eof);
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    pub async fn receive(&mut self) -> Result<Jt809Inbound, Jt809Error> {
        let frame = self.read_frame().await?;
        let inbound = classify(&frame)?;
        match &inbound {
            Jt809Inbound::LoginResponse(response) => {
                self.link.on_login_response(frame.msg_sn, response.result);
            }
            Jt809Inbound::Ack(_) => {
                self.link.acknowledge(frame.msg_sn);
            }
            _ => {}
        }
        Ok(inbound)
    }
}

/// 0x1002 login response subset: result byte + opaque verification tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt809LoginResponse {
    pub result: u8,
    pub tail: Vec<u8>,
}

impl Jt809LoginResponse {
    pub fn decode(body: &[u8]) -> Result<Self, Jt809Error> {
        let Some((&result, tail)) = body.split_first() else {
            return Err(Jt809Error::LoginResponse);
        };
        Ok(Self {
            result,
            tail: tail.to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.tail.len());
        out.push(self.result);
        out.extend_from_slice(&self.tail);
        out
    }
}

fn trim_ascii(bytes: &[u8]) -> Result<String, Jt809Error> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_string)
        .map_err(|_| Jt809Error::LoginBody)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let frame = Jt809Frame {
            msg_id: 0x1001,
            msg_sn: 42,
            encrypt: 0,
            compress: 0,
            body: b"vehicle-snapshot".to_vec(),
        };
        let decoded = Jt809Frame::decode(&frame.encode()).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn login_body_roundtrip() {
        let login = Jt809Login {
            user_id: 1,
            password: "secret".to_string(),
            center_id: 1,
            down_link_ip: "127.0.0.1".to_string(),
            down_link_port: 9001,
        };
        assert_eq!(Jt809Login::decode(&login.encode().unwrap()).unwrap(), login);
    }

    #[tokio::test]
    async fn tcp_connection_login_and_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 256];
            let read = socket.read(&mut buffer).await.unwrap();
            let frame = Jt809Frame::decode(&buffer[..read]).unwrap();
            assert_eq!(frame.msg_id, MSG_LOGIN_REQUEST);
            let response = Jt809Frame {
                msg_id: MSG_LOGIN_RESPONSE,
                msg_sn: frame.msg_sn,
                encrypt: 0,
                compress: 0,
                body: vec![0],
            };
            socket.write_all(&response.encode()).await.unwrap();
        });

        let mut connection = Jt809Connection::connect_tcp(&addr.to_string(), 4)
            .await
            .expect("connect tcp");
        let login = Jt809Login {
            user_id: 1,
            password: "secret".to_string(),
            center_id: 1,
            down_link_ip: "127.0.0.1".to_string(),
            down_link_port: 9001,
        };
        connection.send_login(&login).await.expect("send login");
        let inbound = connection.receive().await.expect("receive");
        assert!(matches!(inbound, Jt809Inbound::LoginResponse(_)));
        assert_eq!(connection.link().state(), Jt809SessionState::Online);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn connect_and_login_rejects_failed_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 256];
            let read = socket.read(&mut buffer).await.unwrap();
            let frame = Jt809Frame::decode(&buffer[..read]).unwrap();
            let response = Jt809Frame {
                msg_id: MSG_LOGIN_RESPONSE,
                msg_sn: frame.msg_sn,
                encrypt: 0,
                compress: 0,
                body: vec![1],
            };
            socket.write_all(&response.encode()).await.unwrap();
        });
        let login = Jt809Login {
            user_id: 1,
            password: "secret".to_string(),
            center_id: 1,
            down_link_ip: "127.0.0.1".to_string(),
            down_link_port: 9001,
        };
        let error = Jt809Connection::connect_and_login(&addr.to_string(), &login, 4)
            .await
            .expect_err("login rejected");
        assert_eq!(error, Jt809Error::LoginRejected(1));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn async_connection_login_and_response() {
        let (client, mut server) = tokio::io::duplex(4096);
        let mut connection = Jt809Connection::new(client, 4);
        let login = Jt809Login {
            user_id: 1,
            password: "secret".to_string(),
            center_id: 1,
            down_link_ip: "127.0.0.1".to_string(),
            down_link_port: 9001,
        };
        let sn = connection.send_login(&login).await.expect("send login");
        assert_eq!(connection.link().state(), Jt809SessionState::LoginSent);

        let mut client_frame = vec![0u8; 128];
        let read = server.read(&mut client_frame).await.expect("server read");
        let decoded = Jt809Frame::decode(&client_frame[..read]).expect("decode login");
        assert_eq!(decoded.msg_id, MSG_LOGIN_REQUEST);
        assert_eq!(Jt809Login::decode(&decoded.body).unwrap(), login);

        let response = Jt809Frame {
            msg_id: MSG_LOGIN_RESPONSE,
            msg_sn: sn,
            encrypt: 0,
            compress: 0,
            body: vec![0],
        };
        server
            .write_all(&response.encode())
            .await
            .expect("server response");
        let inbound = connection.receive().await.expect("receive");
        assert!(matches!(inbound, Jt809Inbound::LoginResponse(_)));
        assert_eq!(connection.link().state(), Jt809SessionState::Online);
    }

    #[test]
    fn inbound_classification_covers_login_ack_business_unknown() {
        let login = Jt809Frame {
            msg_id: MSG_LOGIN_RESPONSE,
            msg_sn: 1,
            encrypt: 0,
            compress: 0,
            body: vec![0],
        };
        assert!(matches!(
            classify(&login),
            Ok(Jt809Inbound::LoginResponse(_))
        ));

        let ack = Jt809Frame {
            msg_id: MSG_PLATFORM_ACK,
            msg_sn: 2,
            encrypt: 0,
            compress: 0,
            body: Jt809Ack {
                ack_sn: 1,
                result: 0,
            }
            .encode(),
        };
        assert!(matches!(classify(&ack), Ok(Jt809Inbound::Ack(_))));

        let business = Jt809Frame {
            msg_id: MSG_UP_DYNAMIC,
            msg_sn: 3,
            encrypt: 0,
            compress: 0,
            body: Jt809BusinessEnvelope {
                sub_type: SUB_REALTIME_LOCATION,
                data: vec![1, 2],
            }
            .encode(),
        };
        assert!(matches!(classify(&business), Ok(Jt809Inbound::Business(_))));

        let unknown = Jt809Frame {
            msg_id: 0x9999,
            msg_sn: 4,
            encrypt: 0,
            compress: 0,
            body: vec![],
        };
        assert_eq!(
            classify(&unknown),
            Ok(Jt809Inbound::Unknown { msg_id: 0x9999 })
        );
    }

    #[test]
    fn reconnect_backoff_is_exponential_and_bounded() {
        let mut backoff =
            Jt809Backoff::new(Duration::from_millis(100), Duration::from_millis(350), 3);
        assert_eq!(backoff.next_delay(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next_delay(), Some(Duration::from_millis(200)));
        assert_eq!(backoff.next_delay(), Some(Duration::from_millis(350)));
        assert_eq!(backoff.next_delay(), None);
        backoff.reset();
        assert_eq!(backoff.next_delay(), Some(Duration::from_millis(100)));
    }

    #[test]
    fn link_lifecycle_login_ack_timeout_reconnect() {
        let mut link = Jt809Link::new(4);
        assert!(link.begin_login(10));
        assert_eq!(link.state(), Jt809SessionState::LoginSent);
        assert!(link.on_login_response(10, 0));
        assert_eq!(link.state(), Jt809SessionState::Online);
        assert_eq!(link.pending_len(), 0);

        assert!(!link.acknowledge(99));
        assert!(!link.begin_login(11));

        link.reconnect();
        assert_eq!(link.state(), Jt809SessionState::Disconnected);
        assert_eq!(link.pending_len(), 0);
    }

    #[test]
    fn retry_window_ack_timeout_and_reset() {
        let mut window = Jt809RetryWindow::new(2);
        assert!(window.mark_sent(1));
        assert!(window.mark_sent(2));
        assert!(!window.mark_sent(3));
        assert_eq!(window.timed_out(Duration::from_secs(0)), vec![1, 2]);
        assert!(window.acknowledge(1));
        assert!(!window.acknowledge(1));
        window.reset();
        assert_eq!(window.pending_len(), 0);
    }

    #[test]
    fn alarm_body_roundtrip_and_range() {
        let alarm = Jt809AlarmData {
            alarm_type: 0x0000_0009,
            lat: 31.2304,
            lon: 121.4737,
            ts: [0x26, 0x11, 0x19, 0x10, 0x11, 0x12],
            tail: vec![0xAA],
        };
        assert_eq!(
            Jt809AlarmData::decode(&alarm.encode().unwrap()).unwrap(),
            alarm
        );
        let mut bad = alarm.encode().unwrap();
        bad[4..8].copy_from_slice(&(910_000_000i32).to_be_bytes());
        assert_eq!(Jt809AlarmData::decode(&bad), Err(Jt809Error::AlarmBody));
    }

    #[test]
    fn platform_ack_roundtrip_and_length() {
        let ack = Jt809Ack {
            ack_sn: 42,
            result: 0,
        };
        assert_eq!(Jt809Ack::decode(&ack.encode()).unwrap(), ack);
        assert_eq!(Jt809Ack::decode(&[0; 4]), Err(Jt809Error::AckBody));
    }

    #[test]
    fn location_body_roundtrip_and_range() {
        let location = Jt809LocationData {
            lat: 31.2304,
            lon: 121.4737,
            altitude_m: 12,
            speed_kmh: 56.3,
            heading: 88,
            ts: [0x26, 0x11, 0x19, 0x10, 0x11, 0x12],
            tail: vec![0xAA],
        };
        assert_eq!(
            Jt809LocationData::decode(&location.encode().unwrap()).unwrap(),
            location
        );
        let mut bad = location.encode().unwrap();
        bad[12] = 0xFF;
        bad[13] = 0xFF;
        assert_eq!(
            Jt809LocationData::decode(&bad),
            Err(Jt809Error::LocationBody)
        );
    }

    #[test]
    fn business_envelope_subtypes_roundtrip() {
        for sub in [
            SUB_REALTIME_LOCATION,
            SUB_LOCATION_REPLAY,
            SUB_LOCATION_SUPPLEMENT,
        ] {
            let envelope = Jt809BusinessEnvelope {
                sub_type: sub,
                data: vec![1, 2, 3],
            };
            assert_eq!(
                Jt809BusinessEnvelope::decode(&envelope.encode()).unwrap(),
                envelope
            );
        }
        assert_eq!(
            Jt809BusinessEnvelope::decode(&[0, 1]),
            Err(Jt809Error::BusinessBody)
        );
    }

    #[test]
    fn encrypted_or_compressed_frames_are_rejected_explicitly() {
        let encrypted = Jt809Frame {
            msg_id: 0x1001,
            msg_sn: 1,
            encrypt: 1,
            compress: 0,
            body: vec![1],
        }
        .encode();
        assert_eq!(Jt809Frame::decode(&encrypted), Err(Jt809Error::Encrypted));

        let compressed = Jt809Frame {
            msg_id: 0x1001,
            msg_sn: 2,
            encrypt: 0,
            compress: 1,
            body: vec![1],
        }
        .encode();
        assert_eq!(Jt809Frame::decode(&compressed), Err(Jt809Error::Compressed));
    }

    #[test]
    fn message_catalog_and_session_handshake() {
        assert_eq!(message_name(0x1001), "platform_login_request");
        let mut session = Jt809Session::new();
        assert_eq!(session.state(), Jt809SessionState::Disconnected);
        assert!(session.on_login_response(0).is_err());
        session.mark_login_sent().expect("login sent");
        session.on_login_response(0).expect("login ok");
        assert_eq!(session.state(), Jt809SessionState::Online);
        session.close();
        assert_eq!(session.state(), Jt809SessionState::Closed);

        let response = Jt809LoginResponse {
            result: 0,
            tail: vec![0xAA, 0xBB],
        };
        assert_eq!(
            Jt809LoginResponse::decode(&response.encode()).unwrap(),
            response
        );
        assert!(Jt809LoginResponse::decode(&[]).is_err());
    }

    #[test]
    fn rejects_bad_crc() {
        let frame = Jt809Frame {
            msg_id: 0x1001,
            msg_sn: 1,
            encrypt: 0,
            compress: 0,
            body: vec![1, 2, 3],
        };
        let mut raw = frame.encode();
        let index = raw.len() - 3;
        raw[index] ^= 0xff;
        assert_eq!(Jt809Frame::decode(&raw), Err(Jt809Error::Crc));
    }

    #[test]
    fn header_2019_layout_roundtrips() {
        let header = Jt809Header2019 {
            msg_length: 128,
            msg_sn: 7,
            msg_id: 0x1001,
            center_id: 0x12345678,
            version: [1, 2, 15],
            encrypt: 0,
            encrypt_key: 0,
            time: 1_700_000_000,
        };
        let raw = header.encode();
        assert_eq!(raw.len(), 30);
        assert_eq!(Jt809Header2019::decode(&raw).unwrap(), header);
        let mut bad = raw;
        bad[17] = 2;
        assert!(Jt809Header2019::decode(&bad).is_err());
    }

    #[test]
    fn frame_2019_escapes_and_roundtrips() {
        let frame = Jt809Frame2019 {
            msg_sn: 9,
            msg_id: 0x1201,
            center_id: 0x12345678,
            version: [1, 2, 15],
            encrypt: 0,
            encrypt_key: 0,
            time: 1_700_000_000,
            body: vec![0x5b, 0x5d, 0x5a, 0x5e, 1, 2, 3],
        };
        let raw = frame.encode().unwrap();
        assert_eq!(raw[0], JT809_START);
        assert_eq!(*raw.last().unwrap(), JT809_END);
        assert_eq!(Jt809Frame2019::decode(&raw).unwrap(), frame);
    }

    #[test]
    fn message_catalog_covers_2019_core_types() {
        assert_eq!(message_name(0x1001), "platform_login_request");
        assert_eq!(message_name(0x9002), "down_connect_response");
        assert_eq!(message_name(0x1200), "up_dynamic_info");
        assert_eq!(message_name(0x120E), "up_drvline");
        assert_eq!(message_name(0x920D), "down_drvline_ack");
        assert_eq!(message_name(0xFFFF), "unknown");
        assert_eq!(MSG_PLATFORM_ACK, 0x9001);
    }

    #[test]
    fn platform_message_envelope_and_ack_roundtrip() {
        let msg = Jt809PlatformMessage {
            data_type: 0x1301,
            data: vec![1, 2, 3],
        };
        let body = msg.encode().unwrap();
        assert_eq!(body.len(), 9);
        assert_eq!(Jt809PlatformMessage::decode(&body).unwrap(), msg);
        let ack = Jt809InfoAck {
            source_data_type: 0x1301,
            source_msg_sn: 42,
        };
        let body = ack.encode();
        assert_eq!(body.len(), 12);
        assert_eq!(Jt809InfoAck::decode(&body).unwrap(), ack);
    }

    #[test]
    fn platform_query_ack_roundtrips() {
        let ack = Jt809PlatformQueryAck {
            object_type: 1,
            responder: "alice".into(),
            responder_tel: "13800000000".into(),
            object_id: "PLATFORM-001".into(),
            source_data_type: 0x9301,
            source_msg_sn: 7,
            info: vec![1, 2, 3],
        };
        let body = ack.encode().unwrap();
        assert_eq!(Jt809PlatformQueryAck::decode(&body).unwrap(), ack);
        assert!(Jt809PlatformQueryAck::decode(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn platform_query_request_roundtrips() {
        let req = Jt809PlatformQueryRequest {
            object_type: 1,
            object_id: "110000-001".into(),
            answer_minutes: 5,
            info_id: 9,
            info: vec![4, 5],
        };
        let body = req.encode().unwrap();
        assert_eq!(body.len(), 38);
        assert_eq!(Jt809PlatformQueryRequest::decode(&body).unwrap(), req);
    }

    #[test]
    fn retransmit_request_roundtrips() {
        let req = Jt809RetransmitRequest {
            data_type: 0x1303,
            link_data_type: 0x1200,
            ranges: vec![(10, 12), (20, 20)],
            time: 1_700_000_000,
        };
        let body = req.encode().unwrap();
        assert_eq!(body.len(), 6 + 27);
        assert_eq!(Jt809RetransmitRequest::decode(&body).unwrap(), req);
        assert!(Jt809RetransmitRequest::decode(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn sequence_tracker_detects_gaps() {
        let mut tracker = Jt809SequenceTracker::new();
        assert_eq!(tracker.next_sn(0x1200), 0);
        assert!(tracker.observe(0x1200, 0).is_empty());
        assert_eq!(tracker.observe(0x1200, 3), vec![(1, 2)]);
        assert_eq!(tracker.next_sn(0x1200), 4);
        assert_eq!(tracker.next_sn(0x1202), 0);
        tracker.observe_retransmit(0x1200, 1);
        assert_eq!(tracker.next_sn(0x1200), 4);
    }

    #[test]
    fn body_cipher_is_configurable_and_self_inverse() {
        let cipher = Jt809Cipher::new(30_000_000, 20_000_000, 20_000_000);
        let original = b"809 encrypted body".to_vec();
        let mut data = original.clone();
        cipher.apply(0x1234, &mut data);
        assert_ne!(data, original);
        cipher.apply(0x1234, &mut data);
        assert_eq!(data, original);
    }

    #[test]
    fn frame_2019_encrypts_body_with_cipher() {
        let cipher = Jt809Cipher::new(30_000_000, 20_000_000, 20_000_000);
        let frame = Jt809Frame2019 {
            msg_sn: 1,
            msg_id: 0x1200,
            center_id: 7,
            version: [1, 0, 0],
            encrypt: 1,
            encrypt_key: 0x1234,
            time: 1_700_000_000,
            body: b"secret body".to_vec(),
        };
        let raw = frame.encode_with_cipher(Some(&cipher)).unwrap();
        assert!(frame.encode().is_err());
        assert_eq!(Jt809Frame2019::decode(&raw), Err(Jt809Error::Encrypted));
        assert_eq!(
            Jt809Frame2019::decode_with_cipher(&raw, Some(&cipher)).unwrap(),
            frame
        );
    }

    #[test]
    fn vehicle_control_ack_roundtrips() {
        let ack = Jt809VehicleControlAck {
            vehicle_no: "粤A12345".into(),
            color: 1,
            subtype: 0x1501,
            source_type: 0x9501,
            source_msg_sn: 3,
            result: 0,
        };
        let body = ack.encode().unwrap();
        assert_eq!(body.len(), 35);
        assert_eq!(Jt809VehicleControlAck::decode(&body).unwrap(), ack);
    }

    #[test]
    fn travel_record_ack_roundtrips() {
        let ack = Jt809TravelRecordAck {
            vehicle_no: "粤A12345".into(),
            color: 1,
            source_type: 0x9504,
            source_msg_sn: 8,
            command_type: 0x01,
            travel_data: vec![1, 2, 3, 4],
        };
        let body = ack.encode().unwrap();
        assert_eq!(Jt809TravelRecordAck::decode(&body).unwrap(), ack);
        assert!(Jt809TravelRecordAck::decode(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn vehicle_control_requests_roundtrip() {
        let photo = Jt809PhotoRequest {
            vehicle_no: "粤A12345".into(),
            color: 1,
            lens_id: 2,
            size: 1,
        };
        assert_eq!(
            Jt809PhotoRequest::decode(&photo.encode().unwrap()).unwrap(),
            photo
        );

        let monitor = Jt809MonitorRequest {
            vehicle_no: "粤A12345".into(),
            color: 1,
            monitor_tel: "13800000000".into(),
        };
        assert_eq!(
            Jt809MonitorRequest::decode(&monitor.encode().unwrap()).unwrap(),
            monitor
        );

        let text = Jt809TextRequest {
            vehicle_no: "粤A12345".into(),
            color: 1,
            priority: 0,
            message: b"hi".to_vec(),
        };
        assert_eq!(
            Jt809TextRequest::decode(&text.encode().unwrap()).unwrap(),
            text
        );

        let travel = Jt809TravelRequest {
            vehicle_no: "粤A12345".into(),
            color: 1,
            start_time: 1_700_000_000,
            end_time: 1_700_000_100,
            command_type: 1,
        };
        assert_eq!(
            Jt809TravelRequest::decode(&travel.encode().unwrap()).unwrap(),
            travel
        );

        let emergency = Jt809EmergencyRequest {
            vehicle_no: "粤A12345".into(),
            color: 1,
            auth_code: [7; 10],
            apn: "apn".into(),
            username: "user".into(),
            password: "pw".into(),
            server_ip: "127.0.0.1".into(),
            tcp_port: 9000,
            udp_port: 9001,
            end_time: 1_700_000_200,
        };
        assert_eq!(
            Jt809EmergencyRequest::decode(&emergency.encode().unwrap()).unwrap(),
            emergency
        );
    }

    #[test]
    fn static_info_exchange_roundtrips() {
        let req = Jt809VehicleStaticRequest {
            vehicle_no: "粤A12345".into(),
            color: 1,
        };
        assert_eq!(
            Jt809VehicleStaticRequest::decode(&req.encode().unwrap()).unwrap(),
            req
        );

        let ack = Jt809VehicleStaticAck {
            vehicle_no: "粤A12345".into(),
            color: 1,
            source_type: 0x9601,
            source_msg_sn: 5,
            car_info: vec![1, 2, 3],
        };
        assert_eq!(
            Jt809VehicleStaticAck::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );

        let route = Jt809DrvLineAck {
            vehicle_no: "粤A12345".into(),
            color: 1,
            route: vec![9, 8, 7],
        };
        assert_eq!(
            Jt809DrvLineAck::decode(&route.encode().unwrap()).unwrap(),
            route
        );
    }

    #[test]
    fn alarm_report_and_result_roundtrip() {
        let report = Jt809AlarmReport {
            platform_id: *b"11000000001",
            warn_type: 1,
            warn_time: 1_700_000_000,
            start_time: 1_700_000_000,
            end_time: 1_700_000_100,
            vehicle_no: "粤A12345".into(),
            vehicle_color: 1,
            target_platform_id: *b"11000000002",
            drvline_id: 3,
            info: vec![1, 2],
        };
        assert_eq!(
            Jt809AlarmReport::decode(&report.encode().unwrap()).unwrap(),
            report
        );

        let ack = Jt809AlarmResultAck {
            source_type: 0x1402,
            source_msg_sn: 9,
            result: 1,
        };
        assert_eq!(
            Jt809AlarmResultAck::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );
    }

    #[test]
    fn photo_ack_with_gnss_roundtrips() {
        let gnss = Jt809GnssBody {
            encrypt: 0,
            gnss: vec![1, 2, 3],
            platform1: [b'1'; 11],
            alarm1: 1,
            platform2: [b'2'; 11],
            alarm2: 2,
            platform3: [b'3'; 11],
            alarm3: 3,
        };
        let ack = Jt809PhotoAck {
            vehicle_no: "粤A12345".into(),
            color: 1,
            source_type: 0x9502,
            source_msg_sn: 7,
            photo_flag: 1,
            gnss,
            lens_id: 2,
            size_type: 1,
            image_type: 1,
            photo: vec![0xFF, 0xD8],
        };
        assert_eq!(Jt809PhotoAck::decode(&ack.encode().unwrap()).unwrap(), ack);
    }

    #[test]
    fn amended_driver_identity_roundtrips() {
        let identity = Jt809DriverIdentity {
            drive_name: "张三".into(),
            driver_id: "110101199001011234".into(),
            licence: "L123456789".into(),
            org_name: "上海市交通委员会".into(),
            valid_date: 1_800_000_000,
            match_score: 95,
            driver_uid: "UID-0001".into(),
        };
        let ack = Jt809DriverIdentityAck {
            vehicle_no: "沪A12345".into(),
            color: 1,
            source_type: 0x920A,
            source_msg_sn: 3,
            identity: identity.clone(),
        };
        assert_eq!(
            Jt809DriverIdentityAck::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );
        let upload = Jt809DriverIdentityUpload {
            vehicle_no: "沪A12345".into(),
            color: 1,
            identity,
        };
        assert_eq!(
            Jt809DriverIdentityUpload::decode(&upload.encode().unwrap()).unwrap(),
            upload
        );
    }

    #[test]
    fn alarm_urge_request_and_ack_roundtrip() {
        let request = Jt809AlarmUrgeRequest {
            platform_id: *b"11000000001",
            warn_time: 1_700_000_000,
            source_type: 0x1402,
            source_msg_sn: 12,
            supervision_deadline: 1_700_003_600,
            supervision_level: 1,
            supervisor: "李四".into(),
            supervisor_tel: "13800138000".into(),
            supervisor_email: "ops@example.com".into(),
        };
        assert_eq!(
            Jt809AlarmUrgeRequest::decode(&request.encode().unwrap()).unwrap(),
            request
        );
        let ack = Jt809AlarmUrgeAck {
            source_type: 0x9401,
            source_msg_sn: 12,
            result: 1,
        };
        assert_eq!(
            Jt809AlarmUrgeAck::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );
    }

    #[test]
    fn alarm_warning_roundtrips() {
        let warning = Jt809AlarmWarning {
            platform_id: *b"11000000001",
            warn_type: 2,
            warn_time: 1_700_000_000,
            start_time: 1_700_000_000,
            end_time: 1_700_000_100,
            vehicle_no: "沪A12345".into(),
            vehicle_color: 1,
            target_platform_id: *b"31000000001",
            drvline_id: 8,
            content: "请立即处理".as_bytes().to_vec(),
        };
        assert_eq!(
            Jt809AlarmWarning::decode(&warning.encode().unwrap()).unwrap(),
            warning
        );
        let mut too_long = warning.clone();
        too_long.content = vec![0; 1025];
        assert!(too_long.encode().is_err());
    }

    #[test]
    fn realtime_alarm_exchange_uses_9403() {
        let warning = Jt809AlarmWarning {
            platform_id: *b"11000000001",
            warn_type: 3,
            warn_time: 1_700_000_000,
            start_time: 1_700_000_000,
            end_time: 1_700_000_100,
            vehicle_no: "沪A12345".into(),
            vehicle_color: 1,
            target_platform_id: *b"31000000001",
            drvline_id: 9,
            content: b"realtime".to_vec(),
        };
        let raw = warning.encode_realtime().unwrap();
        assert_eq!(&raw[..2], &SUB_ALARM_EXCHANGE.to_be_bytes());
        assert_eq!(Jt809AlarmWarning::decode_realtime(&raw).unwrap(), warning);
        assert!(Jt809AlarmWarning::decode(&raw).is_err());
    }

    #[test]
    fn vehicle_static_info_parses_table91_pairs() {
        let (bytes, _, _) =
            encoding_rs::GBK.encode("VIN:=浙A25307;VEHICLE_COLOR:=1;VEHICLE_TYPE:=40;");
        let map = parse_vehicle_static_info(&bytes).unwrap();
        assert_eq!(map.get("VIN").map(String::as_str), Some("浙A25307"));
        assert_eq!(map.get("VEHICLE_COLOR").map(String::as_str), Some("1"));
        assert_eq!(map.get("VEHICLE_TYPE").map(String::as_str), Some("40"));
        assert!(parse_vehicle_static_info(b"NO_SEPARATOR").is_err());
    }

    #[test]
    fn login_and_logout_match_tables_11_and_13() {
        let login = Jt809Login {
            user_id: 42,
            password: "secret".into(),
            center_id: 7,
            down_link_ip: "10.0.0.2".into(),
            down_link_port: 9001,
        };
        let raw = login.encode().unwrap();
        assert_eq!(raw.len(), 50);
        assert_eq!(Jt809Login::decode(&raw).unwrap(), login);
        let logout = Jt809Logout {
            user_id: 42,
            password: "secret".into(),
        };
        let raw = logout.encode().unwrap();
        assert_eq!(raw.len(), 12);
        assert_eq!(Jt809Logout::decode(&raw).unwrap(), logout);
    }

    #[test]
    fn link_notices_roundtrip() {
        assert_eq!(
            Jt809DisconnectNotice::decode(&Jt809DisconnectNotice { error_code: 1 }.encode())
                .unwrap(),
            Jt809DisconnectNotice { error_code: 1 }
        );
        assert_eq!(
            Jt809CloseLinkNotice::decode(&Jt809CloseLinkNotice { reason: 0 }.encode()).unwrap(),
            Jt809CloseLinkNotice { reason: 0 }
        );
        let request = Jt809DownConnectRequest {
            verify_code: 0x11223344,
        };
        assert_eq!(
            Jt809DownConnectRequest::decode(&request.encode()).unwrap(),
            request
        );
    }

    #[test]
    fn vehicle_registration_roundtrips() {
        let mut model = [0u8; 30];
        model[..3].copy_from_slice(b"T10");
        let mut terminal = [0u8; 30];
        terminal[..4].copy_from_slice(b"TERM");
        let record = Jt809VehicleRegister {
            vehicle_no: "沪A12345".into(),
            color: 1,
            platform_id: *b"31000000001",
            producer_id: *b"31000000002",
            terminal_model: model,
            imei: "860000000000001".into(),
            terminal_id: terminal,
            sim_code: "13800138000".into(),
        };
        assert_eq!(
            Jt809VehicleRegister::decode(&record.encode().unwrap()).unwrap(),
            record
        );
    }

    #[test]
    fn return_exchange_acks_roundtrip() {
        let ack = Jt809ReturnAck {
            vehicle_no: "沪A12345".into(),
            color: 1,
            subtype: SUB_RETURN_STARTUP_ACK,
            source_type: 0x9205,
            source_msg_sn: 9,
        };
        assert_eq!(Jt809ReturnAck::decode(&ack.encode().unwrap()).unwrap(), ack);
        let mut end = ack.clone();
        end.subtype = SUB_RETURN_END_ACK;
        assert_eq!(Jt809ReturnAck::decode(&end.encode().unwrap()).unwrap(), end);
    }

    #[test]
    fn monitor_apply_and_cancel_roundtrip() {
        let apply = Jt809MonitorApply {
            vehicle_no: "沪A12345".into(),
            color: 1,
            start_time: 1_700_000_000,
            end_time: 1_700_003_600,
        };
        assert_eq!(
            Jt809MonitorApply::decode(&apply.encode().unwrap()).unwrap(),
            apply
        );
        let cancel = Jt809MonitorCancel {
            vehicle_no: "沪A12345".into(),
            color: 1,
        };
        assert_eq!(
            Jt809MonitorCancel::decode(&cancel.encode().unwrap()).unwrap(),
            cancel
        );
        let bad = Jt809MonitorApply {
            end_time: 1,
            start_time: 2,
            ..apply
        };
        assert!(bad.encode().is_err());
    }

    #[test]
    fn history_replay_request_roundtrips() {
        let request = Jt809HistoryRequest {
            vehicle_no: "沪A12345".into(),
            color: 1,
            start_time: 1_700_000_000,
            end_time: 1_700_007_200,
        };
        assert_eq!(
            Jt809HistoryRequest::decode(&request.encode().unwrap()).unwrap(),
            request
        );
    }

    #[test]
    fn return_startup_and_end_requests_roundtrip() {
        let startup = Jt809ReturnStartupRequest {
            vehicle_no: "沪A12345".into(),
            color: 1,
            reason: 0x02,
        };
        assert_eq!(
            Jt809ReturnStartupRequest::decode(&startup.encode().unwrap()).unwrap(),
            startup
        );
        let end = Jt809ReturnEndRequest {
            vehicle_no: "沪A12345".into(),
            color: 1,
        };
        assert_eq!(
            Jt809ReturnEndRequest::decode(&end.encode().unwrap()).unwrap(),
            end
        );
    }

    #[test]
    fn monitor_acks_roundtrip_with_subtype_results() {
        let apply = Jt809MonitorAck {
            vehicle_no: "沪A12345".into(),
            color: 1,
            subtype: 0x9207,
            source_type: 0x1207,
            source_msg_sn: 4,
            result: 0x02,
        };
        assert_eq!(
            Jt809MonitorAck::decode(&apply.encode().unwrap()).unwrap(),
            apply
        );
        let cancel = Jt809MonitorAck {
            vehicle_no: "沪A12345".into(),
            color: 1,
            subtype: 0x9208,
            source_type: 0x1208,
            source_msg_sn: 5,
            result: 0x01,
        };
        assert_eq!(
            Jt809MonitorAck::decode(&cancel.encode().unwrap()).unwrap(),
            cancel
        );
        let bad = Jt809MonitorAck {
            result: 0x02,
            subtype: 0x9208,
            ..cancel
        };
        assert!(bad.encode().is_err());
    }

    #[test]
    fn history_replay_ack_uses_9209_results() {
        let ack = Jt809MonitorAck {
            vehicle_no: "沪A12345".into(),
            color: 1,
            subtype: 0x9209,
            source_type: 0x1209,
            source_msg_sn: 6,
            result: 0x03,
        };
        assert_eq!(
            Jt809MonitorAck::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );
        let bad = Jt809MonitorAck {
            result: 0x02,
            subtype: 0x9208,
            ..ack
        };
        assert!(bad.encode().is_err());
    }

    #[test]
    fn register_ack_and_static_down_roundtrip() {
        let ack = Jt809RegisterAck {
            source_type: 0x1201,
            source_msg_sn: 11,
            result: 1,
        };
        assert_eq!(
            Jt809RegisterAck::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );
        let down = Jt809VehicleStaticDown {
            vehicle_no: "沪A12345".into(),
            color: 1,
            car_info: "VIN:=LSVAA123456789012;".as_bytes().to_vec(),
        };
        assert_eq!(
            Jt809VehicleStaticDown::decode(&down.encode().unwrap()).unwrap(),
            down
        );
    }

    #[test]
    fn location_down_and_history_down_roundtrip() {
        let gnss = Jt809GnssBody {
            encrypt: 0,
            gnss: vec![1, 2],
            platform1: [b'1'; 11],
            alarm1: 0,
            platform2: [b'2'; 11],
            alarm2: 0,
            platform3: [b'3'; 11],
            alarm3: 0,
        };
        let down = Jt809LocationDown {
            vehicle_no: "沪A12345".into(),
            color: 1,
            gnss: gnss.clone(),
        };
        assert_eq!(
            Jt809LocationDown::decode(&down.encode().unwrap()).unwrap(),
            down
        );
        let history = Jt809HistoryLocationDown {
            vehicle_no: "沪A12345".into(),
            color: 1,
            items: vec![gnss.clone(), gnss],
        };
        assert_eq!(
            Jt809HistoryLocationDown::decode(&history.encode().unwrap()).unwrap(),
            history
        );
    }

    #[test]
    fn driver_ewaybill_and_route_requests_roundtrip() {
        let driver = Jt809DriverInfoRequest {
            vehicle_no: "沪A12345".into(),
            color: 1,
            flag: 1,
        };
        assert_eq!(
            Jt809DriverInfoRequest::decode(&driver.encode().unwrap()).unwrap(),
            driver
        );
        let waybill = Jt809EWaybillRequest {
            vehicle_no: "沪A12345".into(),
            color: 1,
        };
        assert_eq!(
            Jt809EWaybillRequest::decode(&waybill.encode().unwrap()).unwrap(),
            waybill
        );
        let route = Jt809DrvLineRequest {
            vehicle_no: "沪A12345".into(),
            color: 1,
        };
        assert_eq!(
            Jt809DrvLineRequest::decode(&route.encode().unwrap()).unwrap(),
            route
        );
        let ack = Jt809DrvLineAckDown {
            vehicle_no: "沪A12345".into(),
            color: 1,
            drvline_id: 9,
            result: 2,
            reason: "信息错误".into(),
        };
        assert_eq!(
            Jt809DrvLineAckDown::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );
    }

    #[test]
    fn ewaybill_ack_and_upload_roundtrip() {
        let ack = Jt809EWaybillAck {
            vehicle_no: "沪A12345".into(),
            color: 1,
            source_type: 0x920B,
            source_msg_sn: 3,
            bill: vec![1, 2, 3],
        };
        assert_eq!(
            Jt809EWaybillAck::decode(&ack.encode().unwrap()).unwrap(),
            ack
        );
        let up = Jt809EWaybillUpload {
            vehicle_no: "沪A12345".into(),
            color: 1,
            bill: vec![4, 5],
        };
        assert_eq!(
            Jt809EWaybillUpload::decode(&up.encode().unwrap()).unwrap(),
            up
        );
    }

    #[test]
    fn active_route_upload_roundtrips() {
        let upload = Jt809DrvLineUpload {
            vehicle_no: "沪A12345".into(),
            color: 1,
            route: vec![1, 2, 3, 4],
        };
        assert_eq!(
            Jt809DrvLineUpload::decode(&upload.encode().unwrap()).unwrap(),
            upload
        );
    }
}
