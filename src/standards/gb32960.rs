//! GB/T 32960.3-2025 transport-header skeleton.
//!
//! Wire layout implemented here:
//! `0x23 0x23 | command u8 | response u8 | VIN[17] | encrypt u8 | data_len u16 | data`
//!
//! The 0x01 real-time data unit (vehicle status, drive motor, battery, extreme
//! values, alarm codes) and the 0x02/0x03/0x04 command bodies are intentionally
//! not decoded yet. This is a codec seam, not a GB/T 32960 conformance claim.

use thiserror::Error;

pub const GB32960_START: [u8; 2] = [0x24, 0x24];
pub const GB32960_START_2016: [u8; 2] = [0x23, 0x23];
pub const GB32960_LOGIN: u8 = 0x01;
pub const GB32960_REALTIME: u8 = 0x02;
pub const GB32960_SUPPLEMENT: u8 = 0x03;
pub const GB32960_LOGOUT: u8 = 0x04;
pub const GB32960_PLATFORM_LOGIN: u8 = 0x05;
pub const GB32960_PLATFORM_LOGOUT: u8 = 0x06;
pub const GB32960_HEARTBEAT: u8 = 0x07;
pub const GB32960_CLOCK: u8 = 0x08;
pub const GB32960_KEY_EXCHANGE: u8 = 0x0B;
pub const GB32960_ACTIVATION: u8 = 0x09;
pub const GB32960_ACTIVATION_ACK: u8 = 0x0A;

pub const SEC_VEHICLE: u8 = 0x01;
pub const SEC_MOTOR: u8 = 0x02;
pub const SEC_FUEL_CELL: u8 = 0x03;
pub const SEC_ENGINE: u8 = 0x04;
pub const SEC_LOCATION: u8 = 0x05;
pub const SEC_EXTREME: u8 = 0x06;
pub const SEC_ALARM: u8 = 0x07;
pub const SEC_BATTERY_VOLTAGE: u8 = 0x08;
pub const SEC_BATTERY_TEMP: u8 = 0x09;
pub const SEC_SIGNAL: u8 = 0x0A;

// GB/T 32960.3-2025 table 9 section identifiers (differ from 2016).
pub const SEC_2025_ALARM: u8 = 0x06;
pub const SEC_2025_BATTERY_VOLTAGE: u8 = 0x07;
pub const SEC_2025_BATTERY_TEMP: u8 = 0x08;
pub const SEC_2025_FUEL_CELL_STACK: u8 = 0x30;
pub const SEC_2025_SUPERCAP: u8 = 0x31;
pub const SEC_2025_SUPERCAP_EXTREME: u8 = 0x32;
pub const SEC_2025_SIGNATURE: u8 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gb32960Version {
    V2016,
    V2025,
}

impl Gb32960Version {
    pub fn max_alarm_level(self) -> u8 {
        match self {
            Self::V2016 => 3,
            Self::V2025 => 4,
        }
    }

    pub fn validate_alarm_level(self, level: u8) -> Result<(), Gb32960Error> {
        if level <= self.max_alarm_level() {
            Ok(())
        } else {
            Err(Gb32960Error::AlarmBody)
        }
    }
}

/// Logical 32960 section kinds whose wire IDs changed in 2025.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gb32960SectionKind {
    Vehicle,
    Motor,
    FuelCell,
    Engine,
    Location,
    Alarm,
    BatteryVoltage,
    BatteryTemperature,
    FuelCellStack,
    SuperCapacitor,
    SuperCapacitorExtreme,
}

impl Gb32960SectionKind {
    pub fn id(self, version: Gb32960Version) -> Option<u8> {
        match (self, version) {
            (Self::Vehicle, _) => Some(0x01),
            (Self::Motor, _) => Some(0x02),
            (Self::FuelCell, _) => Some(0x03),
            (Self::Engine, _) => Some(0x04),
            (Self::Location, _) => Some(0x05),
            (Self::Alarm, Gb32960Version::V2016) => Some(SEC_ALARM),
            (Self::Alarm, Gb32960Version::V2025) => Some(SEC_2025_ALARM),
            (Self::BatteryVoltage, Gb32960Version::V2016) => Some(SEC_BATTERY_VOLTAGE),
            (Self::BatteryVoltage, Gb32960Version::V2025) => Some(SEC_2025_BATTERY_VOLTAGE),
            (Self::BatteryTemperature, Gb32960Version::V2016) => Some(SEC_BATTERY_TEMP),
            (Self::BatteryTemperature, Gb32960Version::V2025) => Some(SEC_2025_BATTERY_TEMP),
            (Self::FuelCellStack, Gb32960Version::V2025) => Some(SEC_2025_FUEL_CELL_STACK),
            (Self::SuperCapacitor, Gb32960Version::V2025) => Some(SEC_2025_SUPERCAP),
            (Self::SuperCapacitorExtreme, Gb32960Version::V2025) => Some(SEC_2025_SUPERCAP_EXTREME),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Frame {
    pub command: u8,
    pub response: u8,
    pub vin: String,
    pub encrypt: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Gb32960Error {
    #[error("invalid start marker")]
    Start,
    #[error("frame is too short")]
    TooShort,
    #[error("VIN must be 17 ASCII bytes")]
    Vin,
    #[error("length mismatch")]
    Length,
    #[error("real-time data unit shorter than 20 bytes")]
    DataUnit,
    #[error("invalid login body")]
    LoginBody,
    #[error("invalid logout body")]
    LogoutBody,
    #[error("invalid platform ack body")]
    AckBody,
    #[error("invalid alarm body")]
    AlarmBody,
    #[error("invalid location body")]
    LocationBody,
    #[error("invalid battery body")]
    BatteryBody,
    #[error("invalid motor body")]
    MotorBody,
    #[error("encrypted 32960 body is not implemented")]
    Encrypted,
    #[error("invalid extreme body")]
    ExtremeBody,
    #[error("checksum mismatch")]
    Checksum,
    #[error("invalid key exchange body")]
    KeyExchangeBody,
    #[error("cryptographic operation failed")]
    Crypto,
}

impl Gb32960Frame {
    /// Parse 0x02/0x03 with the mandatory 6-byte BCD time prefix.
    pub fn parse_unit_with_time(&self) -> Result<([u8; 6], Gb32960RealtimeUnit), Gb32960Error> {
        if self.command != GB32960_REALTIME && self.command != GB32960_SUPPLEMENT {
            return Err(Gb32960Error::DataUnit);
        }
        if self.data.len() < 6 {
            return Err(Gb32960Error::DataUnit);
        }
        let time = self.data[0..6].try_into().unwrap();
        Ok((time, Gb32960RealtimeUnit::parse(&self.data[6..])?))
    }

    /// Parse 0x02/0x03 data as an ordered section list.
    pub fn parse_unit(&self) -> Result<Gb32960RealtimeUnit, Gb32960Error> {
        if self.command != GB32960_REALTIME && self.command != GB32960_SUPPLEMENT {
            return Err(Gb32960Error::DataUnit);
        }
        Gb32960RealtimeUnit::parse(&self.data)
    }
}
/// TCP stream framer for 0x02/0x03-style frames. The wire carries the data
/// length at bytes 22..24 and a trailing BCC; this decoder buffers partial
/// frames and drops bytes before the next `##` marker.
#[derive(Debug, Default)]
pub struct Gb32960StreamDecoder {
    buffer: Vec<u8>,
}

impl Gb32960StreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Return the next complete, checksum-valid frame.
    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let start = self
                .buffer
                .windows(2)
                .position(|window| window == GB32960_START)
                .or_else(|| {
                    self.buffer
                        .windows(2)
                        .position(|window| window == GB32960_START_2016)
                })?;
            if start > 0 {
                self.buffer.drain(..start);
            }
            if self.buffer.len() < 24 {
                return None;
            }
            let len = u16::from_be_bytes([self.buffer[22], self.buffer[23]]) as usize;
            let total = 24 + len + 1;
            if self.buffer.len() < total {
                return None;
            }
            let frame: Vec<u8> = self.buffer.drain(..total).collect();
            if Gb32960Frame::decode(&frame).is_ok() {
                return Some(frame);
            }
            // Invalid BCC: continue scanning after the marker.
            if self.buffer.len() >= 2 {
                self.buffer.drain(..2);
            }
        }
    }

    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }
}

impl Gb32960Frame {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if !valid_vin(&self.vin) {
            return Err(Gb32960Error::Vin);
        }
        let mut out = Vec::with_capacity(25 + self.data.len());
        out.extend_from_slice(&GB32960_START);
        out.push(self.command);
        out.push(self.response);
        out.extend_from_slice(self.vin.as_bytes());
        out.push(self.encrypt);
        out.extend_from_slice(&(self.data.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.data);
        out.push(bcc(&out[2..]));
        Ok(out)
    }

    /// Decode a frame whose data unit may be encrypted. `decrypt` receives the
    /// encryption method (0x02..0x05) and ciphertext; BCC is verified first.
    pub fn decode_encrypted<F>(raw: &[u8], decrypt: F) -> Result<Self, Gb32960Error>
    where
        F: FnOnce(u8, &[u8]) -> Result<Vec<u8>, Gb32960Error>,
    {
        if raw.len() < 24 {
            return Err(Gb32960Error::TooShort);
        }
        if raw[..2] != GB32960_START && raw[..2] != GB32960_START_2016 {
            return Err(Gb32960Error::Start);
        }
        let command = raw[2];
        let response = raw[3];
        let vin = std::str::from_utf8(&raw[4..21])
            .map_err(|_| Gb32960Error::Vin)?
            .to_string();
        if !valid_vin(&vin) {
            return Err(Gb32960Error::Vin);
        }
        let encrypt = raw[21];
        let len = u16::from_be_bytes([raw[22], raw[23]]) as usize;
        if raw.len() != 24 + len && raw.len() != 25 + len {
            return Err(Gb32960Error::Length);
        }
        if raw.len() == 25 + len && bcc(&raw[2..24 + len]) != raw[24 + len] {
            return Err(Gb32960Error::Checksum);
        }
        let ciphertext = &raw[24..24 + len];
        let data = if encrypt == 0 || encrypt == 0x01 {
            ciphertext.to_vec()
        } else {
            decrypt(encrypt, ciphertext)?
        };
        Ok(Self {
            command,
            response,
            vin,
            encrypt,
            data,
        })
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb32960Error> {
        if raw.len() < 24 {
            return Err(Gb32960Error::TooShort);
        }
        if raw[..2] != GB32960_START && raw[..2] != GB32960_START_2016 {
            return Err(Gb32960Error::Start);
        }
        let command = raw[2];
        let response = raw[3];
        let vin = std::str::from_utf8(&raw[4..21])
            .map_err(|_| Gb32960Error::Vin)?
            .to_string();
        if !valid_vin(&vin) {
            return Err(Gb32960Error::Vin);
        }
        let encrypt = raw[21];
        if matches!(encrypt, 0x02..=0x05) {
            return Err(Gb32960Error::Encrypted);
        }
        if encrypt != 0 && encrypt != 0x01 {
            return Err(Gb32960Error::Encrypted);
        }
        let len = u16::from_be_bytes([raw[22], raw[23]]) as usize;
        if raw.len() != 24 + len && raw.len() != 25 + len {
            return Err(Gb32960Error::Length);
        }
        if raw.len() == 25 + len && bcc(&raw[2..24 + len]) != raw[24 + len] {
            return Err(Gb32960Error::Checksum);
        }
        Ok(Self {
            command,
            response,
            vin,
            encrypt,
            data: raw[24..24 + len].to_vec(),
        })
    }
}

fn bcc(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, byte| acc ^ byte)
}

/// 0x0B data unit encryption key exchange (GB/T 32960.3-2025 table 31).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960KeyExchange {
    pub key_type: u8,
    pub key: Vec<u8>,
    pub enable_time: [u8; 6],
    pub expire_time: [u8; 6],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gb32960RsaDecryptScheme {
    Pkcs1v15,
    OaepSha256,
}

impl Gb32960KeyExchange {
    /// Decrypt the key blob when `key_type == 0x02` (RSA). The caller must
    /// choose the padding; the 2025 text does not specify a default.
    pub fn decrypt_rsa_key(
        &self,
        private_key_pem: &str,
        scheme: Gb32960RsaDecryptScheme,
    ) -> Result<Vec<u8>, Gb32960Error> {
        use rsa::pkcs8::DecodePrivateKey;
        use rsa::RsaPrivateKey;
        if self.key_type != 0x02 {
            return Err(Gb32960Error::Crypto);
        }
        let key =
            RsaPrivateKey::from_pkcs8_pem(private_key_pem).map_err(|_| Gb32960Error::Crypto)?;
        match scheme {
            Gb32960RsaDecryptScheme::Pkcs1v15 => key.decrypt(rsa::Pkcs1v15Encrypt, &self.key),
            Gb32960RsaDecryptScheme::OaepSha256 => {
                key.decrypt(rsa::Oaep::new::<sha2::Sha256>(), &self.key)
            }
        }
        .map_err(|_| Gb32960Error::Crypto)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if !(2..=5).contains(&self.key_type) || self.key.len() > 65531 {
            return Err(Gb32960Error::KeyExchangeBody);
        }
        let mut out = Vec::with_capacity(3 + self.key.len() + 12);
        out.push(self.key_type);
        out.extend_from_slice(&(self.key.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.key);
        out.extend_from_slice(&self.enable_time);
        out.extend_from_slice(&self.expire_time);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 15 {
            return Err(Gb32960Error::KeyExchangeBody);
        }
        let key_type = body[0];
        let len = u16::from_be_bytes([body[1], body[2]]) as usize;
        if !(2..=5).contains(&key_type) || len == 0 || body.len() != 3 + len + 12 {
            return Err(Gb32960Error::KeyExchangeBody);
        }
        Ok(Self {
            key_type,
            key: body[3..3 + len].to_vec(),
            enable_time: body[3 + len..9 + len].try_into().unwrap(),
            expire_time: body[9 + len..15 + len].try_into().unwrap(),
        })
    }
}

/// 0x04 vehicle logout candidate: BCD[6] time + logout serial u16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Logout {
    pub time: [u8; 6],
    pub serial: u16,
}

impl Gb32960Logout {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(&self.time);
        out.extend_from_slice(&self.serial.to_be_bytes());
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 8 {
            return Err(Gb32960Error::LogoutBody);
        }
        Ok(Self {
            time: body[0..6]
                .try_into()
                .map_err(|_| Gb32960Error::LogoutBody)?,
            serial: u16::from_be_bytes([body[6], body[7]]),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gb32960RealtimeData {
    pub vehicle_status: u8,
    pub charge_status: u8,
    pub run_mode: u8,
    pub speed_kmh: f64,
    pub odometer_km: f64,
    pub total_voltage_v: f64,
    pub total_current_a: f64,
    pub soc_percent: u8,
    pub dc_dc_status: u8,
    pub gear: u8,
    pub insulation_kohm: u16,
    pub accel_pedal_percent: u8,
    pub brake_pedal_percent: u8,
    pub tail: Vec<u8>,
}

impl Gb32960RealtimeData {
    pub fn decode(data: &[u8]) -> Result<Self, Gb32960Error> {
        if data.len() < 20 {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self {
            vehicle_status: data[0],
            charge_status: data[1],
            run_mode: data[2],
            speed_kmh: u16::from_be_bytes([data[3], data[4]]) as f64 / 10.0,
            odometer_km: u32::from_be_bytes([data[5], data[6], data[7], data[8]]) as f64 / 10.0,
            total_voltage_v: u16::from_be_bytes([data[9], data[10]]) as f64 / 10.0,
            total_current_a: i16::from_be_bytes([data[11], data[12]]) as f64 / 10.0,
            soc_percent: data[13],
            dc_dc_status: data[14],
            gear: data[15],
            insulation_kohm: u16::from_be_bytes([data[16], data[17]]),
            accel_pedal_percent: data[18],
            brake_pedal_percent: data[19],
            tail: data[20..].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.tail.len());
        out.push(self.vehicle_status);
        out.push(self.charge_status);
        out.push(self.run_mode);
        out.extend_from_slice(&((self.speed_kmh * 10.0) as u16).to_be_bytes());
        out.extend_from_slice(&((self.odometer_km * 10.0) as u32).to_be_bytes());
        out.extend_from_slice(&((self.total_voltage_v * 10.0) as u16).to_be_bytes());
        out.extend_from_slice(&((self.total_current_a * 10.0) as i16).to_be_bytes());
        out.push(self.soc_percent);
        out.push(self.dc_dc_status);
        out.push(self.gear);
        out.extend_from_slice(&self.insulation_kohm.to_be_bytes());
        out.push(self.accel_pedal_percent);
        out.push(self.brake_pedal_percent);
        out.extend_from_slice(&self.tail);
        out
    }
}

/// Login data unit subset: time[6] + serial u16 + user[24] + password[24]
/// 0x05 platform login candidate: BCD[6] time + serial u16 + user[24] + password[24].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960PlatformLogin {
    pub login_time: [u8; 6],
    pub serial: u16,
    pub username: String,
    pub password: String,
}

impl Gb32960PlatformLogin {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        let mut out = Vec::with_capacity(56);
        out.extend_from_slice(&self.login_time);
        out.extend_from_slice(&self.serial.to_be_bytes());
        out.extend_from_slice(&fixed_24(&self.username)?);
        out.extend_from_slice(&fixed_24(&self.password)?);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 56 {
            return Err(Gb32960Error::LoginBody);
        }
        Ok(Self {
            login_time: body[0..6].try_into().map_err(|_| Gb32960Error::LoginBody)?,
            serial: u16::from_be_bytes([body[6], body[7]]),
            username: trim_24(&body[8..32])?,
            password: trim_24(&body[32..56])?,
        })
    }
}

fn fixed_24(value: &str) -> Result<[u8; 24], Gb32960Error> {
    if !value.is_ascii() || value.len() > 24 {
        return Err(Gb32960Error::LoginBody);
    }
    let mut out = [0u8; 24];
    out[..value.len()].copy_from_slice(value.as_bytes());
    Ok(out)
}

fn trim_24(bytes: &[u8]) -> Result<String, Gb32960Error> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_string)
        .map_err(|_| Gb32960Error::LoginBody)
}

/// 0x06 extreme-value section candidate (14 bytes, raw values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960ExtremeData {
    pub max_voltage_subsystem: u8,
    pub max_voltage_cell: u8,
    pub max_voltage_raw: u16,
    pub min_voltage_subsystem: u8,
    pub min_voltage_cell: u8,
    pub min_voltage_raw: u16,
    pub max_temp_subsystem: u8,
    pub max_temp_probe: u8,
    pub max_temp_raw: u8,
    pub min_temp_subsystem: u8,
    pub min_temp_probe: u8,
    pub min_temp_raw: u8,
}

impl Gb32960ExtremeData {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(14);
        out.push(self.max_voltage_subsystem);
        out.push(self.max_voltage_cell);
        out.extend_from_slice(&self.max_voltage_raw.to_be_bytes());
        out.push(self.min_voltage_subsystem);
        out.push(self.min_voltage_cell);
        out.extend_from_slice(&self.min_voltage_raw.to_be_bytes());
        out.push(self.max_temp_subsystem);
        out.push(self.max_temp_probe);
        out.push(self.max_temp_raw);
        out.push(self.min_temp_subsystem);
        out.push(self.min_temp_probe);
        out.push(self.min_temp_raw);
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 14 {
            return Err(Gb32960Error::ExtremeBody);
        }
        Ok(Self {
            max_voltage_subsystem: body[0],
            max_voltage_cell: body[1],
            max_voltage_raw: u16::from_be_bytes([body[2], body[3]]),
            min_voltage_subsystem: body[4],
            min_voltage_cell: body[5],
            min_voltage_raw: u16::from_be_bytes([body[6], body[7]]),
            max_temp_subsystem: body[8],
            max_temp_probe: body[9],
            max_temp_raw: body[10],
            min_temp_subsystem: body[11],
            min_temp_probe: body[12],
            min_temp_raw: body[13],
        })
    }
}

/// One 2025-candidate drive motor record (8 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Motor {
    pub serial: u8,
    pub status: u8,
    pub controller_temp_raw: u8,
    pub speed_raw: u16,
    pub torque_raw: u16,
    pub temp_raw: u8,
}

impl Gb32960Motor {
    fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.serial);
        out.push(self.status);
        out.push(self.controller_temp_raw);
        out.extend_from_slice(&self.speed_raw.to_be_bytes());
        out.extend_from_slice(&self.torque_raw.to_be_bytes());
        out.push(self.temp_raw);
    }

    fn decode_at(body: &[u8], offset: usize) -> Result<Self, Gb32960Error> {
        Ok(Self {
            serial: body[offset],
            status: body[offset + 1],
            controller_temp_raw: body[offset + 2],
            speed_raw: u16::from_be_bytes([body[offset + 3], body[offset + 4]]),
            torque_raw: u16::from_be_bytes([body[offset + 5], body[offset + 6]]),
            temp_raw: body[offset + 7],
        })
    }
}

/// 0x02 drive motor section candidate: count u8 + 8-byte records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960MotorData {
    pub motors: Vec<Gb32960Motor>,
}

impl Gb32960MotorData {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.motors.len() > u8::MAX as usize {
            return Err(Gb32960Error::MotorBody);
        }
        let mut out = Vec::with_capacity(1 + self.motors.len() * 8);
        out.push(self.motors.len() as u8);
        for motor in &self.motors {
            motor.encode_into(&mut out);
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.is_empty() {
            return Err(Gb32960Error::MotorBody);
        }
        let count = body[0] as usize;
        if body.len() != 1 + count * 8 {
            return Err(Gb32960Error::MotorBody);
        }
        let mut motors = Vec::with_capacity(count);
        for index in 0..count {
            motors.push(Gb32960Motor::decode_at(body, 1 + index * 8)?);
        }
        Ok(Self { motors })
    }
}

/// 0x08 battery voltage section candidate: subsystem u8 + cell count u16 + mV list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960BatteryVoltage {
    pub subsystem: u8,
    pub cell_mv: Vec<u16>,
}

impl Gb32960BatteryVoltage {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.cell_mv.len() > u16::MAX as usize {
            return Err(Gb32960Error::BatteryBody);
        }
        let mut out = Vec::with_capacity(3 + self.cell_mv.len() * 2);
        out.push(self.subsystem);
        out.extend_from_slice(&(self.cell_mv.len() as u16).to_be_bytes());
        for value in &self.cell_mv {
            out.extend_from_slice(&value.to_be_bytes());
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 3 {
            return Err(Gb32960Error::BatteryBody);
        }
        let count = u16::from_be_bytes([body[1], body[2]]) as usize;
        if body.len() != 3 + count * 2 {
            return Err(Gb32960Error::BatteryBody);
        }
        let mut cell_mv = Vec::with_capacity(count);
        for index in 0..count {
            let offset = 3 + index * 2;
            cell_mv.push(u16::from_be_bytes([body[offset], body[offset + 1]]));
        }
        Ok(Self {
            subsystem: body[0],
            cell_mv,
        })
    }
}

/// 0x09 battery temperature section candidate: subsystem u8 + probe count u16 + raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960BatteryTemperature {
    pub subsystem: u8,
    pub probes: Vec<u8>,
}

impl Gb32960BatteryTemperature {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.probes.len() > u16::MAX as usize {
            return Err(Gb32960Error::BatteryBody);
        }
        let mut out = Vec::with_capacity(3 + self.probes.len());
        out.push(self.subsystem);
        out.extend_from_slice(&(self.probes.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.probes);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 3 {
            return Err(Gb32960Error::BatteryBody);
        }
        let count = u16::from_be_bytes([body[1], body[2]]) as usize;
        if body.len() != 3 + count {
            return Err(Gb32960Error::BatteryBody);
        }
        Ok(Self {
            subsystem: body[0],
            probes: body[3..].to_vec(),
        })
    }
}

/// 0x05 location section candidate: status u8 + lon/lat i32 /1e6 + altitude u16.
#[derive(Debug, Clone, PartialEq)]
pub struct Gb32960LocationData {
    pub status: u8,
    pub lon: f64,
    pub lat: f64,
    pub altitude_m: u16,
}

impl Gb32960LocationData {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if !(-90.0..=90.0).contains(&self.lat) || !(-180.0..=180.0).contains(&self.lon) {
            return Err(Gb32960Error::LocationBody);
        }
        let mut out = Vec::with_capacity(11);
        out.push(self.status);
        out.extend_from_slice(&((self.lon * 1_000_000.0) as i32).to_be_bytes());
        out.extend_from_slice(&((self.lat * 1_000_000.0) as i32).to_be_bytes());
        out.extend_from_slice(&self.altitude_m.to_be_bytes());
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 11 {
            return Err(Gb32960Error::LocationBody);
        }
        let lon = i32::from_be_bytes([body[1], body[2], body[3], body[4]]) as f64 / 1_000_000.0;
        let lat = i32::from_be_bytes([body[5], body[6], body[7], body[8]]) as f64 / 1_000_000.0;
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Err(Gb32960Error::LocationBody);
        }
        Ok(Self {
            status: body[0],
            lon,
            lat,
            altitude_m: u16::from_be_bytes([body[9], body[10]]),
        })
    }
}

/// GB/T 32960.3-2025 platform login (table 29).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960PlatformLogin2025 {
    pub login_time: [u8; 6],
    pub serial: u16,
    pub username: String,
    pub password: String,
    pub encryption: u8,
}

impl Gb32960PlatformLogin2025 {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if !(1..=5).contains(&self.encryption) {
            return Err(Gb32960Error::LoginBody);
        }
        let mut out = Vec::with_capacity(41);
        out.extend_from_slice(&self.login_time);
        out.extend_from_slice(&self.serial.to_be_bytes());
        out.extend_from_slice(&fixed_ascii(&self.username, 12)?);
        out.extend_from_slice(&fixed_ascii(&self.password, 20)?);
        out.push(self.encryption);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 41 || !(1..=5).contains(&body[40]) {
            return Err(Gb32960Error::LoginBody);
        }
        let (username, _, u) = encoding_rs::GBK.decode(trim_nul(&body[8..20]));
        let (password, _, p) = encoding_rs::GBK.decode(trim_nul(&body[20..40]));
        if u || p {
            return Err(Gb32960Error::LoginBody);
        }
        Ok(Self {
            login_time: body[0..6].try_into().unwrap(),
            serial: u16::from_be_bytes([body[6], body[7]]),
            username: username.into_owned(),
            password: password.into_owned(),
            encryption: body[40],
        })
    }
}

/// 2025 table 8: vehicle-side signature (R/S values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Signature {
    pub sign_type: u8,
    pub r: Vec<u8>,
    pub s: Vec<u8>,
}

impl Gb32960Signature {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if !(1..=3).contains(&self.sign_type) || self.r.len() > 65531 || self.s.len() > 65531 {
            return Err(Gb32960Error::DataUnit);
        }
        let mut out = Vec::with_capacity(5 + self.r.len() + self.s.len());
        out.push(self.sign_type);
        out.extend_from_slice(&(self.r.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.r);
        out.extend_from_slice(&(self.s.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.s);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 5 {
            return Err(Gb32960Error::DataUnit);
        }
        let sign_type = body[0];
        if !(1..=3).contains(&sign_type) {
            return Err(Gb32960Error::DataUnit);
        }
        let r_len = u16::from_be_bytes([body[1], body[2]]) as usize;
        if body.len() < 5 + r_len {
            return Err(Gb32960Error::DataUnit);
        }
        let r = body[3..3 + r_len].to_vec();
        let s_len_at = 3 + r_len;
        let s_len = u16::from_be_bytes([body[s_len_at], body[s_len_at + 1]]) as usize;
        let s_at = s_len_at + 2;
        if body.len() != s_at + s_len {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self {
            sign_type,
            r,
            s: body[s_at..].to_vec(),
        })
    }
}

/// SM4-CBC/PKCS#7. The 2025 text names SM4 but not a mode; this helper is
/// explicit and does not claim to be the standard default.
pub fn encrypt_sm4_cbc(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, Gb32960Error> {
    use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    if key.len() != 16 || iv.len() != 16 {
        return Err(Gb32960Error::Crypto);
    }
    let encryptor = cbc::Encryptor::<sm4::Sm4>::new(key.into(), iv.into());
    Ok(encryptor.encrypt_padded_vec_mut::<Pkcs7>(plaintext))
}

pub fn decrypt_sm4_cbc(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, Gb32960Error> {
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    if key.len() != 16
        || iv.len() != 16
        || ciphertext.is_empty()
        || !ciphertext.len().is_multiple_of(16)
    {
        return Err(Gb32960Error::Crypto);
    }
    let decryptor = cbc::Decryptor::<sm4::Sm4>::new(key.into(), iv.into());
    decryptor
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| Gb32960Error::Crypto)
}

/// AES-128 modes are explicit; GB/T 32960.3-2025 names AES but does not
/// define the mode/IV/padding in the text used here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gb32960AesScheme {
    CbcPkcs7,
    Gcm,
}

pub fn encrypt_aes_128(
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8],
    scheme: Gb32960AesScheme,
) -> Result<Vec<u8>, Gb32960Error> {
    use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    use aes_gcm::aead::{Aead, KeyInit};
    match scheme {
        Gb32960AesScheme::CbcPkcs7 => {
            if key.len() != 16 || iv.len() != 16 {
                return Err(Gb32960Error::Crypto);
            }
            let encryptor = cbc::Encryptor::<aes::Aes128>::new(key.into(), iv.into());
            Ok(encryptor.encrypt_padded_vec_mut::<Pkcs7>(plaintext))
        }
        Gb32960AesScheme::Gcm => {
            if key.len() != 16 || iv.len() != 12 {
                return Err(Gb32960Error::Crypto);
            }
            let cipher =
                aes_gcm::Aes128Gcm::new_from_slice(key).map_err(|_| Gb32960Error::Crypto)?;
            cipher
                .encrypt(iv.into(), plaintext)
                .map_err(|_| Gb32960Error::Crypto)
        }
    }
}

pub fn decrypt_aes_128(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    scheme: Gb32960AesScheme,
) -> Result<Vec<u8>, Gb32960Error> {
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    use aes_gcm::aead::{Aead, KeyInit};
    match scheme {
        Gb32960AesScheme::CbcPkcs7 => {
            if key.len() != 16
                || iv.len() != 16
                || ciphertext.is_empty()
                || !ciphertext.len().is_multiple_of(16)
            {
                return Err(Gb32960Error::Crypto);
            }
            let decryptor = cbc::Decryptor::<aes::Aes128>::new(key.into(), iv.into());
            decryptor
                .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
                .map_err(|_| Gb32960Error::Crypto)
        }
        Gb32960AesScheme::Gcm => {
            if key.len() != 16 || iv.len() != 12 {
                return Err(Gb32960Error::Crypto);
            }
            let cipher =
                aes_gcm::Aes128Gcm::new_from_slice(key).map_err(|_| Gb32960Error::Crypto)?;
            cipher
                .decrypt(iv.into(), ciphertext)
                .map_err(|_| Gb32960Error::Crypto)
        }
    }
}

/// RSA signature schemes are explicit: the 2025 text names "RSA" but not
/// the padding, so callers must choose instead of relying on a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gb32960RsaScheme {
    Pkcs1v15Sha256,
    PssSha256,
}

impl Gb32960Signature {
    /// Verify an RSA signature over `data`. RSA signatures are carried in `r`;
    /// `s` is accepted for symmetry with SM2/ECC and must be empty.
    pub fn verify_rsa(
        &self,
        data: &[u8],
        public_key_pem: &str,
        scheme: Gb32960RsaScheme,
    ) -> Result<bool, Gb32960Error> {
        use rsa::pkcs8::DecodePublicKey;
        let key = rsa::RsaPublicKey::from_public_key_pem(public_key_pem)
            .map_err(|_| Gb32960Error::Crypto)?;
        self.verify_rsa_with_key(data, &key, scheme)
    }

    /// Verify with a pre-parsed key so callers can cache it across frames.
    pub fn verify_rsa_with_key(
        &self,
        data: &[u8],
        key: &rsa::RsaPublicKey,
        scheme: Gb32960RsaScheme,
    ) -> Result<bool, Gb32960Error> {
        use rsa::pkcs1v15::Signature as Pkcs1Signature;
        use rsa::signature::Verifier;
        if self.sign_type != 0x02 || !self.s.is_empty() {
            return Err(Gb32960Error::Crypto);
        }
        let key = key.clone();
        match scheme {
            Gb32960RsaScheme::Pkcs1v15Sha256 => {
                let verifier = rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(key);
                let signature = Pkcs1Signature::try_from(self.r.as_slice())
                    .map_err(|_| Gb32960Error::Crypto)?;
                Ok(verifier.verify(data, &signature).is_ok())
            }
            Gb32960RsaScheme::PssSha256 => {
                let verifier = rsa::pss::VerifyingKey::<sha2::Sha256>::new(key);
                let signature = rsa::pss::Signature::try_from(self.r.as_slice())
                    .map_err(|_| Gb32960Error::Crypto)?;
                Ok(verifier.verify(data, &signature).is_ok())
            }
        }
    }
}

impl Gb32960Signature {
    /// Verify an SM2 signature over `data` (SM3 + user identifier ZA).
    /// `public_key_sec1` is an uncompressed SEC1 point (0x04 || X || Y).
    pub fn verify_sm2(
        &self,
        data: &[u8],
        public_key_sec1: &[u8],
        distid: &str,
    ) -> Result<bool, Gb32960Error> {
        let key = sm2::dsa::VerifyingKey::from_sec1_bytes(distid, public_key_sec1)
            .map_err(|_| Gb32960Error::Crypto)?;
        self.verify_sm2_with_key(data, &key)
    }

    /// Verify with a pre-built key (ZA already computed).
    pub fn verify_sm2_with_key(
        &self,
        data: &[u8],
        key: &sm2::dsa::VerifyingKey,
    ) -> Result<bool, Gb32960Error> {
        use sm2::dsa::signature::Verifier;
        use sm2::dsa::Signature as Sm2Signature;
        if self.sign_type != 0x01 || self.r.len() != 32 || self.s.len() != 32 {
            return Err(Gb32960Error::Crypto);
        }
        let mut raw = Vec::with_capacity(64);
        raw.extend_from_slice(&self.r);
        raw.extend_from_slice(&self.s);
        let signature = Sm2Signature::from_slice(&raw).map_err(|_| Gb32960Error::Crypto)?;
        Ok(key.verify(data, &signature).is_ok())
    }
}

impl Gb32960Signature {
    /// Verify an ECDSA P-256/SHA-256 signature. R/S are 32 bytes each and the
    /// public key is an uncompressed SEC1 point.
    pub fn verify_ecdsa_p256(
        &self,
        data: &[u8],
        public_key_sec1: &[u8],
    ) -> Result<bool, Gb32960Error> {
        use p256::ecdsa::signature::Verifier;
        use p256::ecdsa::{Signature as P256Signature, VerifyingKey};
        if self.sign_type != 0x03 || self.r.len() != 32 || self.s.len() != 32 {
            return Err(Gb32960Error::Crypto);
        }
        let key =
            VerifyingKey::from_sec1_bytes(public_key_sec1).map_err(|_| Gb32960Error::Crypto)?;
        use p256::elliptic_curve::generic_array::GenericArray;
        let r = GenericArray::clone_from_slice(&self.r);
        let s = GenericArray::clone_from_slice(&self.s);
        let signature = P256Signature::from_scalars(r, s).map_err(|_| Gb32960Error::Crypto)?;
        Ok(key.verify(data, &signature).is_ok())
    }
}

/// 2025 5.2.4: after drive/charge ends and high voltage is removed, sample
/// battery voltage/temperature at >=1 Hz for one hour and report at least one
/// record in the 59m30s..1h window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gb32960HighVoltageTrigger {
    DriveEnd,
    ChargeEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960HighVoltageMonitor {
    pub trigger: Gb32960HighVoltageTrigger,
    pub started_at: u64,
    pub last_sample_at: Option<u64>,
    pub samples: u64,
    pub reported: bool,
}

impl Gb32960HighVoltageMonitor {
    pub fn start(trigger: Gb32960HighVoltageTrigger, now: u64) -> Self {
        Self {
            trigger,
            started_at: now,
            last_sample_at: None,
            samples: 0,
            reported: false,
        }
    }

    pub fn is_active(&self, now: u64) -> bool {
        now >= self.started_at && now <= self.started_at + 3600
    }

    /// Returns true when a new sample is due (>=1 Hz, capped to one per second).
    pub fn tick(&mut self, now: u64) -> bool {
        if !self.is_active(now) {
            return false;
        }
        if let Some(last) = self.last_sample_at {
            if now.saturating_sub(last) < 1 {
                return false;
            }
        }
        self.last_sample_at = Some(now);
        self.samples += 1;
        true
    }

    /// True once in the 59m30s..1h report window.
    pub fn report_due(&self, now: u64) -> bool {
        !self.reported && now >= self.started_at + 3570 && now <= self.started_at + 3600
    }

    pub fn mark_reported(&mut self) {
        self.reported = true;
    }
}

/// SM3 digest (GB/T 32905-2016).
pub fn sm3_hash(data: &[u8]) -> [u8; 32] {
    use sm3::{Digest, Sm3};
    let mut hasher = Sm3::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// SM2 ciphertext layout is not fixed by the 32960 text; callers must choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gb32960Sm2CipherEncoding {
    C1C3C2,
    C1C2C3,
}

/// Decrypt SM2 ciphertext with a 32-byte private scalar.
pub fn decrypt_sm2(
    private_key: &[u8; 32],
    ciphertext: &[u8],
    encoding: Gb32960Sm2CipherEncoding,
) -> Result<Vec<u8>, Gb32960Error> {
    use sm2::elliptic_curve::sec1::FromEncodedPoint;
    use sm2::elliptic_curve::PrimeField;
    use sm2::{AffinePoint, EncodedPoint, FieldBytes, ProjectivePoint, Scalar};
    use sm3::{Digest, Sm3};

    if ciphertext.len() < 65 + 32 + 1 {
        return Err(Gb32960Error::Crypto);
    }
    let c1_owned;
    let (c1_bytes, rest) = if ciphertext[0] == 0x04 {
        (&ciphertext[..65], &ciphertext[65..])
    } else {
        let mut with_prefix = Vec::with_capacity(65);
        with_prefix.push(0x04);
        with_prefix.extend_from_slice(&ciphertext[..64]);
        c1_owned = with_prefix;
        (c1_owned.as_slice(), &ciphertext[64..])
    };
    let point = EncodedPoint::from_bytes(c1_bytes).map_err(|_| Gb32960Error::Crypto)?;
    let c1 = AffinePoint::from_encoded_point(&point)
        .into_option()
        .ok_or(Gb32960Error::Crypto)?;
    let scalar = Scalar::from_repr(FieldBytes::clone_from_slice(private_key))
        .into_option()
        .ok_or(Gb32960Error::Crypto)?;
    let shared = (ProjectivePoint::from(c1) * scalar).to_affine();
    use sm2::elliptic_curve::sec1::ToEncodedPoint;
    let encoded = shared.to_encoded_point(false);
    let x2 = encoded.x().ok_or(Gb32960Error::Crypto)?.to_vec();
    let y2 = encoded.y().ok_or(Gb32960Error::Crypto)?.to_vec();

    let (c3, c2) = match encoding {
        Gb32960Sm2CipherEncoding::C1C3C2 => (&rest[..32], &rest[32..]),
        Gb32960Sm2CipherEncoding::C1C2C3 => (&rest[rest.len() - 32..], &rest[..rest.len() - 32]),
    };
    if c2.is_empty() {
        return Err(Gb32960Error::Crypto);
    }
    let mut z = Vec::with_capacity(64);
    z.extend_from_slice(&x2);
    z.extend_from_slice(&y2);
    let mask = sm2_kdf(&z, c2.len())?;
    let mut plain = Vec::with_capacity(c2.len());
    for (a, b) in c2.iter().zip(mask.iter()) {
        plain.push(a ^ b);
    }
    let mut hasher = Sm3::new();
    hasher.update(&x2);
    hasher.update(&plain);
    hasher.update(&y2);
    let digest = hasher.finalize();
    if digest.as_slice() != c3 {
        return Err(Gb32960Error::Crypto);
    }
    Ok(plain)
}

fn sm2_kdf(z: &[u8], len: usize) -> Result<Vec<u8>, Gb32960Error> {
    use sm3::{Digest, Sm3};
    if len > 65535 {
        return Err(Gb32960Error::Crypto);
    }
    let mut out = Vec::with_capacity(len);
    let mut counter: u32 = 1;
    while out.len() < len {
        let mut hasher = Sm3::new();
        hasher.update(z);
        hasher.update(counter.to_be_bytes());
        out.extend_from_slice(&hasher.finalize());
        counter = counter.checked_add(1).ok_or(Gb32960Error::Crypto)?;
    }
    out.truncate(len);
    if out.iter().all(|b| *b == 0) {
        return Err(Gb32960Error::Crypto);
    }
    Ok(out)
}

/// SM2 ciphertext layout is not fixed by the 32960 text; callers must choose.
/// 2025 table B.3: activation information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Activation {
    pub time: [u8; 6],
    pub chip_id: String,
    pub public_key: Vec<u8>,
    pub vin: String,
    pub signature: Gb32960Signature,
}

impl Gb32960Activation {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if !valid_vin(&self.vin) {
            return Err(Gb32960Error::Vin);
        }
        let chip = fixed_ascii(&self.chip_id, 16)?;
        if self.public_key.len() > 65531 {
            return Err(Gb32960Error::DataUnit);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&self.time);
        out.extend_from_slice(&chip);
        out.extend_from_slice(&(self.public_key.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.public_key);
        out.extend_from_slice(self.vin.as_bytes());
        out.extend_from_slice(&self.signature.encode()?);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 6 + 16 + 2 + 17 {
            return Err(Gb32960Error::DataUnit);
        }
        let time = body[0..6].try_into().unwrap();
        let (chip_id, _, bad) = encoding_rs::GBK.decode(trim_nul(&body[6..22]));
        if bad {
            return Err(Gb32960Error::DataUnit);
        }
        let key_len = u16::from_be_bytes([body[22], body[23]]) as usize;
        let key_at = 24;
        if body.len() < key_at + key_len + 17 {
            return Err(Gb32960Error::DataUnit);
        }
        let public_key = body[key_at..key_at + key_len].to_vec();
        let vin_at = key_at + key_len;
        let vin = std::str::from_utf8(&body[vin_at..vin_at + 17])
            .map_err(|_| Gb32960Error::Vin)?
            .to_string();
        if !valid_vin(&vin) {
            return Err(Gb32960Error::Vin);
        }
        let signature = Gb32960Signature::decode(&body[vin_at + 17..])?;
        Ok(Self {
            time,
            chip_id: chip_id.into_owned(),
            public_key,
            vin,
            signature,
        })
    }
}

/// 2025 table B.4: activation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960ActivationAck {
    pub status: u8,
    pub info: u8,
}

impl Gb32960ActivationAck {
    pub fn encode(&self) -> Vec<u8> {
        vec![self.status, self.info]
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 2 {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self {
            status: body[0],
            info: body[1],
        })
    }
}

/// 2025 table 27: custom data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960CustomData {
    pub data: Vec<u8>,
}

impl Gb32960CustomData {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.data.is_empty() || self.data.len() > 65531 {
            return Err(Gb32960Error::DataUnit);
        }
        let mut out = Vec::with_capacity(2 + self.data.len());
        out.extend_from_slice(&(self.data.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.data);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 2 {
            return Err(Gb32960Error::DataUnit);
        }
        let len = u16::from_be_bytes([body[0], body[1]]) as usize;
        if len == 0 || body.len() != 2 + len {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self {
            data: body[2..].to_vec(),
        })
    }
}

/// GB/T 32960.3-2025 vehicle login (table 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Login2025 {
    pub time: [u8; 6],
    pub serial: u16,
    pub iccid: String,
    pub bms_pack_counts: Vec<u8>,
    pub pack_codes: Vec<String>,
}

impl Gb32960Login2025 {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.bms_pack_counts.len() > 20 {
            return Err(Gb32960Error::LoginBody);
        }
        let total: usize = self.bms_pack_counts.iter().map(|n| *n as usize).sum();
        if self.pack_codes.len() != total {
            return Err(Gb32960Error::LoginBody);
        }
        let iccid = fixed_ascii(&self.iccid, 20)?;
        let mut out = Vec::with_capacity(29 + self.bms_pack_counts.len() + total * 24);
        out.extend_from_slice(&self.time);
        out.extend_from_slice(&self.serial.to_be_bytes());
        out.extend_from_slice(&iccid);
        out.push(self.bms_pack_counts.len() as u8);
        out.extend_from_slice(&self.bms_pack_counts);
        for code in &self.pack_codes {
            out.extend_from_slice(&fixed_ascii(code, 24)?);
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 29 {
            return Err(Gb32960Error::LoginBody);
        }
        let time = body[0..6].try_into().map_err(|_| Gb32960Error::LoginBody)?;
        let serial = u16::from_be_bytes([body[6], body[7]]);
        let (iccid, _, bad) = encoding_rs::GBK.decode(trim_nul(&body[8..28]));
        if bad {
            return Err(Gb32960Error::LoginBody);
        }
        let bms_count = body[28] as usize;
        let mut offset = 29;
        if body.len() < offset + bms_count {
            return Err(Gb32960Error::LoginBody);
        }
        let bms_pack_counts = body[offset..offset + bms_count].to_vec();
        offset += bms_count;
        let total: usize = bms_pack_counts.iter().map(|n| *n as usize).sum();
        if body.len() != offset + total * 24 {
            return Err(Gb32960Error::LoginBody);
        }
        let mut pack_codes = Vec::with_capacity(total);
        for _ in 0..total {
            let (code, _, bad) = encoding_rs::GBK.decode(trim_nul(&body[offset..offset + 24]));
            if bad {
                return Err(Gb32960Error::LoginBody);
            }
            pack_codes.push(code.into_owned());
            offset += 24;
        }
        Ok(Self {
            time,
            serial,
            iccid: iccid.into_owned(),
            bms_pack_counts,
            pack_codes,
        })
    }
}

fn trim_nul(data: &[u8]) -> &[u8] {
    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    &data[..end]
}

fn fixed_ascii(value: &str, size: usize) -> Result<Vec<u8>, Gb32960Error> {
    if !value.is_ascii() || value.len() > size {
        return Err(Gb32960Error::LoginBody);
    }
    let mut out = vec![0u8; size];
    out[..value.len()].copy_from_slice(value.as_bytes());
    Ok(out)
}

/// 2025 0x30: fuel-cell stack data (tables 18/19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960FuelCellStack {
    pub no: u8,
    pub voltage: u16,
    pub current: u16,
    pub hydrogen_inlet_pressure: u16,
    pub air_inlet_pressure: u16,
    pub air_inlet_temp: u8,
    pub coolant_temps: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960FuelCellStacks {
    pub stacks: Vec<Gb32960FuelCellStack>,
}

impl Gb32960FuelCellStacks {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.stacks.len() > 253 {
            return Err(Gb32960Error::DataUnit);
        }
        let mut out = vec![self.stacks.len() as u8];
        for stack in &self.stacks {
            if stack.coolant_temps.len() > 65531 {
                return Err(Gb32960Error::DataUnit);
            }
            out.push(stack.no);
            out.extend_from_slice(&stack.voltage.to_be_bytes());
            out.extend_from_slice(&stack.current.to_be_bytes());
            out.extend_from_slice(&stack.hydrogen_inlet_pressure.to_be_bytes());
            out.extend_from_slice(&stack.air_inlet_pressure.to_be_bytes());
            out.push(stack.air_inlet_temp);
            out.extend_from_slice(&(stack.coolant_temps.len() as u16).to_be_bytes());
            out.extend_from_slice(&stack.coolant_temps);
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.is_empty() {
            return Err(Gb32960Error::DataUnit);
        }
        let count = body[0] as usize;
        let mut offset = 1;
        let mut stacks = Vec::with_capacity(count);
        for _ in 0..count {
            if body.len() < offset + 12 {
                return Err(Gb32960Error::DataUnit);
            }
            let no = body[offset];
            let voltage = u16::from_be_bytes([body[offset + 1], body[offset + 2]]);
            let current = u16::from_be_bytes([body[offset + 3], body[offset + 4]]);
            let hydrogen_inlet_pressure = u16::from_be_bytes([body[offset + 5], body[offset + 6]]);
            let air_inlet_pressure = u16::from_be_bytes([body[offset + 7], body[offset + 8]]);
            let air_inlet_temp = body[offset + 9];
            let probes = u16::from_be_bytes([body[offset + 10], body[offset + 11]]) as usize;
            offset += 12;
            if body.len() < offset + probes {
                return Err(Gb32960Error::DataUnit);
            }
            stacks.push(Gb32960FuelCellStack {
                no,
                voltage,
                current,
                hydrogen_inlet_pressure,
                air_inlet_pressure,
                air_inlet_temp,
                coolant_temps: body[offset..offset + probes].to_vec(),
            });
            offset += probes;
        }
        if offset != body.len() {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self { stacks })
    }
}

/// 2025 0x31: supercapacitor data (table 25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960SuperCapacitor {
    pub system_no: u8,
    pub total_voltage: u16,
    pub total_current: u16,
    pub cell_voltages: Vec<u16>,
    pub temperatures: Vec<u8>,
}

impl Gb32960SuperCapacitor {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.cell_voltages.is_empty() || self.temperatures.is_empty() {
            return Err(Gb32960Error::DataUnit);
        }
        let mut out = Vec::new();
        out.push(self.system_no);
        out.extend_from_slice(&self.total_voltage.to_be_bytes());
        out.extend_from_slice(&self.total_current.to_be_bytes());
        out.extend_from_slice(&(self.cell_voltages.len() as u16).to_be_bytes());
        for value in &self.cell_voltages {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(&(self.temperatures.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.temperatures);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 11 {
            return Err(Gb32960Error::DataUnit);
        }
        let system_no = body[0];
        let total_voltage = u16::from_be_bytes([body[1], body[2]]);
        let total_current = u16::from_be_bytes([body[3], body[4]]);
        let cells = u16::from_be_bytes([body[5], body[6]]) as usize;
        if cells == 0 || body.len() < 7 + cells * 2 + 2 {
            return Err(Gb32960Error::DataUnit);
        }
        let mut offset = 7;
        let mut cell_voltages = Vec::with_capacity(cells);
        for _ in 0..cells {
            cell_voltages.push(u16::from_be_bytes([body[offset], body[offset + 1]]));
            offset += 2;
        }
        let probes = u16::from_be_bytes([body[offset], body[offset + 1]]) as usize;
        offset += 2;
        if probes == 0 || body.len() != offset + probes {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self {
            system_no,
            total_voltage,
            total_current,
            cell_voltages,
            temperatures: body[offset..].to_vec(),
        })
    }
}

/// 2025 0x32: supercapacitor extreme values (table 26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960SuperCapacitorExtreme {
    pub max_voltage_system: u8,
    pub max_voltage_cell: u16,
    pub max_voltage: u16,
    pub min_voltage_system: u8,
    pub min_voltage_cell: u16,
    pub min_voltage: u16,
    pub max_temp_system: u8,
    pub max_temp_probe: u16,
    pub max_temp: u8,
    pub min_temp_system: u8,
    pub min_temp_probe: u16,
    pub min_temp: u8,
}

impl Gb32960SuperCapacitorExtreme {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(18);
        out.push(self.max_voltage_system);
        out.extend_from_slice(&self.max_voltage_cell.to_be_bytes());
        out.extend_from_slice(&self.max_voltage.to_be_bytes());
        out.push(self.min_voltage_system);
        out.extend_from_slice(&self.min_voltage_cell.to_be_bytes());
        out.extend_from_slice(&self.min_voltage.to_be_bytes());
        out.push(self.max_temp_system);
        out.extend_from_slice(&self.max_temp_probe.to_be_bytes());
        out.push(self.max_temp);
        out.push(self.min_temp_system);
        out.extend_from_slice(&self.min_temp_probe.to_be_bytes());
        out.push(self.min_temp);
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 18 {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self {
            max_voltage_system: body[0],
            max_voltage_cell: u16::from_be_bytes([body[1], body[2]]),
            max_voltage: u16::from_be_bytes([body[3], body[4]]),
            min_voltage_system: body[5],
            min_voltage_cell: u16::from_be_bytes([body[6], body[7]]),
            min_voltage: u16::from_be_bytes([body[8], body[9]]),
            max_temp_system: body[10],
            max_temp_probe: u16::from_be_bytes([body[11], body[12]]),
            max_temp: body[13],
            min_temp_system: body[14],
            min_temp_probe: u16::from_be_bytes([body[15], body[16]]),
            min_temp: body[17],
        })
    }
}

/// 2025 0x07: battery minimum parallel unit voltage data (tables 11/12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960BatteryVoltage2025Pack {
    pub pack_no: u8,
    pub pack_voltage: u16,
    pub pack_current: u16,
    pub unit_voltages: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960BatteryVoltage2025 {
    pub packs: Vec<Gb32960BatteryVoltage2025Pack>,
}

impl Gb32960BatteryVoltage2025 {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.packs.len() > 50 {
            return Err(Gb32960Error::BatteryBody);
        }
        let mut out = Vec::new();
        out.push(self.packs.len() as u8);
        for pack in &self.packs {
            if pack.unit_voltages.is_empty() || pack.unit_voltages.len() > 65531 {
                return Err(Gb32960Error::BatteryBody);
            }
            out.push(pack.pack_no);
            out.extend_from_slice(&pack.pack_voltage.to_be_bytes());
            out.extend_from_slice(&pack.pack_current.to_be_bytes());
            out.extend_from_slice(&(pack.unit_voltages.len() as u16).to_be_bytes());
            for value in &pack.unit_voltages {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.is_empty() {
            return Err(Gb32960Error::BatteryBody);
        }
        let count = body[0] as usize;
        let mut offset = 1;
        let mut packs = Vec::with_capacity(count);
        for _ in 0..count {
            if body.len() < offset + 7 {
                return Err(Gb32960Error::BatteryBody);
            }
            let pack_no = body[offset];
            let pack_voltage = u16::from_be_bytes([body[offset + 1], body[offset + 2]]);
            let pack_current = u16::from_be_bytes([body[offset + 3], body[offset + 4]]);
            let units = u16::from_be_bytes([body[offset + 5], body[offset + 6]]) as usize;
            offset += 7;
            if units == 0 || body.len() < offset + units * 2 {
                return Err(Gb32960Error::BatteryBody);
            }
            let mut unit_voltages = Vec::with_capacity(units);
            for _ in 0..units {
                unit_voltages.push(u16::from_be_bytes([body[offset], body[offset + 1]]));
                offset += 2;
            }
            packs.push(Gb32960BatteryVoltage2025Pack {
                pack_no,
                pack_voltage,
                pack_current,
                unit_voltages,
            });
        }
        if offset != body.len() {
            return Err(Gb32960Error::BatteryBody);
        }
        Ok(Self { packs })
    }
}

/// 0x08 platform time synchronization: BCD YY-MM-DD-hh-mm-ss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gb32960Clock {
    pub time: [u8; 6],
}

impl Gb32960Clock {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        valid_bcd_time(&self.time)?;
        Ok(self.time.to_vec())
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 6 {
            return Err(Gb32960Error::DataUnit);
        }
        let time: [u8; 6] = body.try_into().unwrap();
        valid_bcd_time(&time)?;
        Ok(Self { time })
    }
}

fn valid_bcd_time(time: &[u8; 6]) -> Result<(), Gb32960Error> {
    for byte in time {
        if byte >> 4 > 9 || byte & 0x0F > 9 {
            return Err(Gb32960Error::DataUnit);
        }
    }
    Ok(())
}

/// 0x04 engine data: status + crank speed + fuel consumption rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gb32960EngineData {
    pub status: u8,
    pub crank_speed_rpm: u16,
    pub fuel_rate_raw: u16,
}

impl Gb32960EngineData {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5);
        out.push(self.status);
        out.extend_from_slice(&self.crank_speed_rpm.to_be_bytes());
        out.extend_from_slice(&self.fuel_rate_raw.to_be_bytes());
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 5 {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self {
            status: body[0],
            crank_speed_rpm: u16::from_be_bytes([body[1], body[2]]),
            fuel_rate_raw: u16::from_be_bytes([body[3], body[4]]),
        })
    }
}

/// 0x0A vehicle signal state byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gb32960SignalState {
    pub status: u8,
}

impl Gb32960SignalState {
    pub fn encode(&self) -> Vec<u8> {
        vec![self.status]
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 1 {
            return Err(Gb32960Error::DataUnit);
        }
        Ok(Self { status: body[0] })
    }
}

/// 2025 0x08: battery temperature data (tables 13/14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960BatteryTemp2025Pack {
    pub pack_no: u8,
    pub temperatures: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960BatteryTemp2025 {
    pub packs: Vec<Gb32960BatteryTemp2025Pack>,
}

impl Gb32960BatteryTemp2025 {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.packs.len() > 50 {
            return Err(Gb32960Error::BatteryBody);
        }
        let mut out = Vec::new();
        out.push(self.packs.len() as u8);
        for pack in &self.packs {
            if pack.temperatures.is_empty() || pack.temperatures.len() > 65531 {
                return Err(Gb32960Error::BatteryBody);
            }
            out.push(pack.pack_no);
            out.extend_from_slice(&(pack.temperatures.len() as u16).to_be_bytes());
            out.extend_from_slice(&pack.temperatures);
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.is_empty() {
            return Err(Gb32960Error::BatteryBody);
        }
        let count = body[0] as usize;
        let mut offset = 1;
        let mut packs = Vec::with_capacity(count);
        for _ in 0..count {
            if body.len() < offset + 3 {
                return Err(Gb32960Error::BatteryBody);
            }
            let pack_no = body[offset];
            let probes = u16::from_be_bytes([body[offset + 1], body[offset + 2]]) as usize;
            offset += 3;
            if probes == 0 || body.len() < offset + probes {
                return Err(Gb32960Error::BatteryBody);
            }
            packs.push(Gb32960BatteryTemp2025Pack {
                pack_no,
                temperatures: body[offset..offset + probes].to_vec(),
            });
            offset += probes;
        }
        if offset != body.len() {
            return Err(Gb32960Error::BatteryBody);
        }
        Ok(Self { packs })
    }
}

/// 0x07 alarm section candidate: level u8 + flags u32 + four fault-code lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960AlarmData {
    pub level: u8,
    pub flags: u32,
    pub battery_faults: Vec<u32>,
    pub motor_faults: Vec<u32>,
    pub engine_faults: Vec<u32>,
    pub other_faults: Vec<u32>,
    /// 2025 N5: (general alarm flag bit index, alarm level).
    pub general_alarms: Vec<(u8, u8)>,
}

impl Gb32960AlarmData {
    pub fn validate_level(&self, version: Gb32960Version) -> Result<(), Gb32960Error> {
        version.validate_alarm_level(self.level)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        self.encode_version(Gb32960Version::V2025)
    }

    pub fn encode_version(&self, version: Gb32960Version) -> Result<Vec<u8>, Gb32960Error> {
        let mut out = Vec::new();
        out.push(self.level);
        out.extend_from_slice(&self.flags.to_be_bytes());
        for faults in [
            &self.battery_faults,
            &self.motor_faults,
            &self.engine_faults,
            &self.other_faults,
        ] {
            if faults.len() > u8::MAX as usize {
                return Err(Gb32960Error::AlarmBody);
            }
            out.push(faults.len() as u8);
            for fault in faults {
                out.extend_from_slice(&fault.to_be_bytes());
            }
        }
        if version == Gb32960Version::V2025 {
            if self.general_alarms.len() > u8::MAX as usize {
                return Err(Gb32960Error::AlarmBody);
            }
            out.push(self.general_alarms.len() as u8);
            for (flag, level) in &self.general_alarms {
                out.push(*flag);
                out.push(*level);
            }
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 9 {
            return Err(Gb32960Error::AlarmBody);
        }
        let level = body[0];
        let flags = u32::from_be_bytes([body[1], body[2], body[3], body[4]]);
        let mut index = 5;
        let mut lists = Vec::with_capacity(4);
        for _ in 0..4 {
            if index >= body.len() {
                return Err(Gb32960Error::AlarmBody);
            }
            let count = body[index] as usize;
            index += 1;
            if index + count * 4 > body.len() {
                return Err(Gb32960Error::AlarmBody);
            }
            let mut list = Vec::with_capacity(count);
            for _ in 0..count {
                list.push(u32::from_be_bytes([
                    body[index],
                    body[index + 1],
                    body[index + 2],
                    body[index + 3],
                ]));
                index += 4;
            }
            lists.push(list);
        }
        let mut general_alarms = Vec::new();
        if index < body.len() {
            let count = body[index] as usize;
            index += 1;
            if index + count * 2 != body.len() {
                return Err(Gb32960Error::AlarmBody);
            }
            for _ in 0..count {
                general_alarms.push((body[index], body[index + 1]));
                index += 2;
            }
        }
        if index != body.len() {
            return Err(Gb32960Error::AlarmBody);
        }
        Ok(Self {
            level,
            flags,
            battery_faults: lists[0].clone(),
            motor_faults: lists[1].clone(),
            engine_faults: lists[2].clone(),
            other_faults: lists[3].clone(),
            general_alarms,
        })
    }
}

/// Platform response candidate: BCD[6] time + response serial u16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960PlatformAck {
    pub time: [u8; 6],
    pub serial: u16,
}

impl Gb32960PlatformAck {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(&self.time);
        out.extend_from_slice(&self.serial.to_be_bytes());
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() != 8 {
            return Err(Gb32960Error::AckBody);
        }
        Ok(Self {
            time: body[0..6].try_into().map_err(|_| Gb32960Error::AckBody)?,
            serial: u16::from_be_bytes([body[6], body[7]]),
        })
    }
}

/// One ordered 32960 information section: type(u8) + length(u16) + data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Section {
    pub data_type: u8,
    pub data: Vec<u8>,
}

impl Gb32960Section {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(3 + self.data.len());
        out.push(self.data_type);
        out.extend_from_slice(&(self.data.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode_all(body: &[u8]) -> Result<Vec<Self>, Gb32960Error> {
        let mut out = Vec::with_capacity((body.len() / 32).clamp(4, 64));
        let mut index = 0;
        while index < body.len() {
            if body.len() < index + 3 {
                return Err(Gb32960Error::DataUnit);
            }
            let data_type = body[index];
            let len = u16::from_be_bytes([body[index + 1], body[index + 2]]) as usize;
            index += 3;
            if body.len() < index + len {
                return Err(Gb32960Error::DataUnit);
            }
            out.push(Self {
                data_type,
                data: body[index..index + len].to_vec(),
            });
            index += len;
        }
        Ok(out)
    }

    pub fn vehicle_data(&self) -> Result<Gb32960RealtimeData, Gb32960Error> {
        if self.data_type != SEC_VEHICLE {
            return Err(Gb32960Error::DataUnit);
        }
        Gb32960RealtimeData::decode(&self.data)
    }
}

/// Parsed 0x02 real-time unit: ordered sections plus typed accessors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960RealtimeUnit {
    pub sections: Vec<Gb32960Section>,
}

impl Gb32960RealtimeUnit {
    pub fn parse(data: &[u8]) -> Result<Self, Gb32960Error> {
        Ok(Self {
            sections: Gb32960Section::decode_all(data)?,
        })
    }

    pub fn section(&self, data_type: u8) -> Option<&Gb32960Section> {
        self.sections
            .iter()
            .find(|section| section.data_type == data_type)
    }

    pub fn vehicle(&self) -> Result<Gb32960RealtimeData, Gb32960Error> {
        self.section(SEC_VEHICLE)
            .ok_or(Gb32960Error::DataUnit)?
            .vehicle_data()
    }

    pub fn location(&self) -> Result<Gb32960LocationData, Gb32960Error> {
        Gb32960LocationData::decode(
            &self
                .section(SEC_LOCATION)
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    pub fn motor(&self) -> Result<Gb32960MotorData, Gb32960Error> {
        Gb32960MotorData::decode(&self.section(SEC_MOTOR).ok_or(Gb32960Error::DataUnit)?.data)
    }

    pub fn engine(&self) -> Result<Gb32960EngineData, Gb32960Error> {
        Gb32960EngineData::decode(&self.section(SEC_ENGINE).ok_or(Gb32960Error::DataUnit)?.data)
    }

    pub fn signal_state(&self) -> Result<Gb32960SignalState, Gb32960Error> {
        Gb32960SignalState::decode(&self.section(SEC_SIGNAL).ok_or(Gb32960Error::DataUnit)?.data)
    }

    pub fn alarm(&self) -> Result<Gb32960AlarmData, Gb32960Error> {
        Gb32960AlarmData::decode(
            &self
                .section(SEC_2025_ALARM)
                .or_else(|| self.section(SEC_ALARM))
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    pub fn battery_voltage_2025(&self) -> Result<Gb32960BatteryVoltage2025, Gb32960Error> {
        Gb32960BatteryVoltage2025::decode(
            &self
                .section(SEC_2025_BATTERY_VOLTAGE)
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    pub fn battery_temperature_2025(&self) -> Result<Gb32960BatteryTemp2025, Gb32960Error> {
        Gb32960BatteryTemp2025::decode(
            &self
                .section(SEC_2025_BATTERY_TEMP)
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    pub fn fuel_cell_stacks(&self) -> Result<Gb32960FuelCellStacks, Gb32960Error> {
        Gb32960FuelCellStacks::decode(
            &self
                .section(SEC_2025_FUEL_CELL_STACK)
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    pub fn supercapacitor(&self) -> Result<Gb32960SuperCapacitor, Gb32960Error> {
        Gb32960SuperCapacitor::decode(
            &self
                .section(SEC_2025_SUPERCAP)
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    pub fn supercapacitor_extreme(&self) -> Result<Gb32960SuperCapacitorExtreme, Gb32960Error> {
        Gb32960SuperCapacitorExtreme::decode(
            &self
                .section(SEC_2025_SUPERCAP_EXTREME)
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    pub fn signature(&self) -> Result<Gb32960Signature, Gb32960Error> {
        Gb32960Signature::decode(
            &self
                .section(SEC_2025_SIGNATURE)
                .ok_or(Gb32960Error::DataUnit)?
                .data,
        )
    }

    /// Validate ids and ordering for a declared protocol version.
    pub fn validate_sections(&self, version: Gb32960Version) -> Result<(), Gb32960Error> {
        let order: &[u8] = match version {
            Gb32960Version::V2016 => &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A],
            Gb32960Version::V2025 => &[
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x30, 0x31, 0x32, 0xFF,
            ],
        };
        let mut last = None;
        for section in &self.sections {
            let rank = order
                .iter()
                .position(|id| *id == section.data_type)
                .ok_or(Gb32960Error::DataUnit)?;
            if last.is_some_and(|previous| rank <= previous) {
                return Err(Gb32960Error::DataUnit);
            }
            last = Some(rank);
        }
        Ok(())
    }

    /// Reject duplicate section ids in one data unit.
    pub fn validate_unique_sections(&self) -> Result<(), Gb32960Error> {
        let mut seen = std::collections::BTreeSet::new();
        for section in &self.sections {
            if !seen.insert(section.data_type) {
                return Err(Gb32960Error::DataUnit);
            }
        }
        Ok(())
    }
}

/// 0x01 vehicle login candidate: BCD[6] time + serial u16 + ICCID[20]
/// + subsystem count u8 + code length u8 + concatenated subsystem codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb32960Login {
    pub login_time: [u8; 6],
    pub serial: u16,
    pub iccid: String,
    pub subsystems: Vec<String>,
}

impl Gb32960Login {
    pub fn encode(&self) -> Result<Vec<u8>, Gb32960Error> {
        if self.iccid.len() != 20 || !self.iccid.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Gb32960Error::LoginBody);
        }
        let code_len = self.subsystems.iter().map(String::len).max().unwrap_or(0);
        if code_len > u8::MAX as usize || self.subsystems.len() > u8::MAX as usize {
            return Err(Gb32960Error::LoginBody);
        }
        let mut out = Vec::with_capacity(30 + code_len * self.subsystems.len());
        out.extend_from_slice(&self.login_time);
        out.extend_from_slice(&self.serial.to_be_bytes());
        out.extend_from_slice(self.iccid.as_bytes());
        out.push(self.subsystems.len() as u8);
        out.push(code_len as u8);
        for code in &self.subsystems {
            if !code.is_ascii() || code.len() > code_len {
                return Err(Gb32960Error::LoginBody);
            }
            let mut fixed = vec![0u8; code_len];
            fixed[..code.len()].copy_from_slice(code.as_bytes());
            out.extend_from_slice(&fixed);
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Gb32960Error> {
        if body.len() < 30 {
            return Err(Gb32960Error::LoginBody);
        }
        let iccid = std::str::from_utf8(&body[8..28])
            .map_err(|_| Gb32960Error::LoginBody)?
            .trim_matches('\0')
            .to_string();
        if iccid.len() != 20 || !iccid.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Gb32960Error::LoginBody);
        }
        let count = body[28] as usize;
        let code_len = body[29] as usize;
        let expected = 30 + count * code_len;
        if body.len() != expected {
            return Err(Gb32960Error::LoginBody);
        }
        let mut subsystems = Vec::with_capacity(count);
        for index in 0..count {
            let offset = 30 + index * code_len;
            let raw = &body[offset..offset + code_len];
            let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
            let code = std::str::from_utf8(&raw[..end])
                .map_err(|_| Gb32960Error::LoginBody)?
                .to_string();
            subsystems.push(code);
        }
        Ok(Self {
            login_time: body[0..6].try_into().map_err(|_| Gb32960Error::LoginBody)?,
            serial: u16::from_be_bytes([body[6], body[7]]),
            iccid,
            subsystems,
        })
    }
}

fn valid_vin(value: &str) -> bool {
    value.len() == 17 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let frame = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0xfe,
            vin: "LSVAA2A41NX000001".to_string(),
            encrypt: 0,
            data: vec![1, 2, 3, 4],
        };
        assert_eq!(
            Gb32960Frame::decode(&frame.encode().unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn realtime_data_unit_subset_roundtrip() {
        let unit = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 2,
            run_mode: 3,
            speed_kmh: 42.5,
            odometer_km: 12345.6,
            total_voltage_v: 380.5,
            total_current_a: -12.3,
            soc_percent: 76,
            dc_dc_status: 1,
            gear: 4,
            insulation_kohm: 550,
            accel_pedal_percent: 20,
            brake_pedal_percent: 0,
            tail: vec![0xaa, 0xbb],
        };
        let decoded = Gb32960RealtimeData::decode(&unit.encode()).unwrap();
        assert_eq!(decoded, unit);
    }

    #[test]
    fn rejects_short_realtime_unit() {
        assert_eq!(
            Gb32960RealtimeData::decode(&[0; 19]),
            Err(Gb32960Error::DataUnit)
        );
    }

    #[test]
    fn login_body_roundtrip() {
        let login = Gb32960Login {
            login_time: [0x26, 0x11, 0x16, 0x10, 0x00, 0x00],
            serial: 9,
            iccid: "89860012345678901234".to_string(),
            subsystems: vec!["BAT001".to_string(), "BAT002".to_string()],
        };
        assert_eq!(
            Gb32960Login::decode(&login.encode().unwrap()).unwrap(),
            login
        );
    }

    #[test]
    fn platform_login_and_logout_bodies() {
        let login = Gb32960PlatformLogin {
            login_time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
            serial: 3,
            username: "platform".to_string(),
            password: "secret".to_string(),
        };
        assert_eq!(
            Gb32960PlatformLogin::decode(&login.encode().unwrap()).unwrap(),
            login
        );
        assert_eq!(
            Gb32960PlatformLogin::decode(&[0; 55]),
            Err(Gb32960Error::LoginBody)
        );

        let logout = Gb32960Logout {
            time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
            serial: 3,
        };
        let frame = Gb32960Frame {
            command: GB32960_PLATFORM_LOGOUT,
            response: 0,
            vin: "LSVAA123456789012".to_string(),
            encrypt: 0,
            data: logout.encode(),
        };
        let decoded = Gb32960Frame::decode(&frame.encode().unwrap()).unwrap();
        assert_eq!(decoded.command, GB32960_PLATFORM_LOGOUT);
        assert_eq!(Gb32960Logout::decode(&decoded.data).unwrap(), logout);
    }

    #[test]
    fn encrypted_frame_is_rejected_explicitly() {
        let frame = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0,
            vin: "LSVAA123456789012".to_string(),
            encrypt: 3,
            data: vec![1, 2, 3],
        };
        let raw = frame.encode().unwrap();
        assert_eq!(Gb32960Frame::decode(&raw), Err(Gb32960Error::Encrypted));
    }

    #[test]
    fn extreme_section_roundtrip_and_length() {
        let extreme = Gb32960ExtremeData {
            max_voltage_subsystem: 1,
            max_voltage_cell: 2,
            max_voltage_raw: 3600,
            min_voltage_subsystem: 1,
            min_voltage_cell: 3,
            min_voltage_raw: 3100,
            max_temp_subsystem: 1,
            max_temp_probe: 4,
            max_temp_raw: 80,
            min_temp_subsystem: 1,
            min_temp_probe: 5,
            min_temp_raw: 45,
        };
        assert_eq!(
            Gb32960ExtremeData::decode(&extreme.encode()).unwrap(),
            extreme
        );
        assert_eq!(
            Gb32960ExtremeData::decode(&[0; 13]),
            Err(Gb32960Error::ExtremeBody)
        );
    }

    #[test]
    fn motor_section_roundtrip_and_count_check() {
        let data = Gb32960MotorData {
            motors: vec![
                Gb32960Motor {
                    serial: 1,
                    status: 1,
                    controller_temp_raw: 40,
                    speed_raw: 3000,
                    torque_raw: 2000,
                    temp_raw: 45,
                },
                Gb32960Motor {
                    serial: 2,
                    status: 3,
                    controller_temp_raw: 50,
                    speed_raw: 0,
                    torque_raw: 0,
                    temp_raw: 55,
                },
            ],
        };
        assert_eq!(
            Gb32960MotorData::decode(&data.encode().unwrap()).unwrap(),
            data
        );
        assert_eq!(
            Gb32960MotorData::decode(&[1, 1, 1, 40, 0, 0, 0, 0]),
            Err(Gb32960Error::MotorBody)
        );
    }

    #[test]
    fn battery_voltage_and_temperature_roundtrip() {
        let voltage = Gb32960BatteryVoltage {
            subsystem: 1,
            cell_mv: vec![3200, 3300, 3400],
        };
        assert_eq!(
            Gb32960BatteryVoltage::decode(&voltage.encode().unwrap()).unwrap(),
            voltage
        );
        assert_eq!(
            Gb32960BatteryVoltage::decode(&[1, 0, 2, 0]),
            Err(Gb32960Error::BatteryBody)
        );

        let temperature = Gb32960BatteryTemperature {
            subsystem: 1,
            probes: vec![40, 41, 255],
        };
        assert_eq!(
            Gb32960BatteryTemperature::decode(&temperature.encode().unwrap()).unwrap(),
            temperature
        );
        assert_eq!(
            Gb32960BatteryTemperature::decode(&[1, 0, 3, 40]),
            Err(Gb32960Error::BatteryBody)
        );
    }

    #[test]
    fn location_section_roundtrip_and_range() {
        let location = Gb32960LocationData {
            status: 0b0000_0001,
            lon: 121.4737,
            lat: 31.2304,
            altitude_m: 12,
        };
        assert_eq!(
            Gb32960LocationData::decode(&location.encode().unwrap()).unwrap(),
            location
        );
        let mut bad = location.encode().unwrap();
        bad[5..9].copy_from_slice(&(910_000_000i32).to_be_bytes());
        assert_eq!(
            Gb32960LocationData::decode(&bad),
            Err(Gb32960Error::LocationBody)
        );
    }

    #[test]
    fn alarm_section_roundtrip_and_reject_truncated() {
        let alarm = Gb32960AlarmData {
            level: 3,
            flags: 0x0000_0011,
            battery_faults: vec![1, 2],
            motor_faults: vec![3],
            engine_faults: vec![],
            other_faults: vec![4],
            general_alarms: vec![],
        };
        assert_eq!(
            Gb32960AlarmData::decode(&alarm.encode().unwrap()).unwrap(),
            alarm
        );
        let mut raw = alarm.encode().unwrap();
        raw.pop();
        raw.pop();
        assert_eq!(Gb32960AlarmData::decode(&raw), Err(Gb32960Error::AlarmBody));
    }

    #[test]
    fn supplement_frame_reuses_section_container() {
        let section = Gb32960Section {
            data_type: SEC_VEHICLE,
            data: vec![0u8; 20],
        };
        let frame = Gb32960Frame {
            command: GB32960_SUPPLEMENT,
            response: 0,
            vin: "LSVAA123456789012".to_string(),
            encrypt: 0,
            data: section.encode(),
        };
        let raw = frame.encode().unwrap();
        let decoded = Gb32960Frame::decode(&raw).unwrap();
        let unit = decoded.parse_unit().unwrap();
        assert_eq!(unit.section(SEC_VEHICLE).unwrap().data.len(), 20);
        let heartbeat = Gb32960Frame {
            command: GB32960_HEARTBEAT,
            response: 0,
            vin: "LSVAA123456789012".to_string(),
            encrypt: 0,
            data: vec![],
        };
        assert_eq!(heartbeat.parse_unit(), Err(Gb32960Error::DataUnit));
    }

    #[test]
    fn platform_ack_and_realtime_unit_accessors() {
        let ack = Gb32960PlatformAck {
            time: [0x26, 0x11, 0x19, 0x10, 0x11, 0x12],
            serial: 7,
        };
        assert_eq!(Gb32960PlatformAck::decode(&ack.encode()).unwrap(), ack);
        assert_eq!(
            Gb32960PlatformAck::decode(&[0; 7]),
            Err(Gb32960Error::AckBody)
        );

        let vehicle = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 2,
            run_mode: 3,
            speed_kmh: 1.0,
            odometer_km: 2.0,
            total_voltage_v: 3.0,
            total_current_a: 4.0,
            soc_percent: 5,
            dc_dc_status: 6,
            gear: 7,
            insulation_kohm: 8,
            accel_pedal_percent: 9,
            brake_pedal_percent: 10,
            tail: vec![],
        };
        let data = Gb32960Section {
            data_type: SEC_VEHICLE,
            data: vehicle.encode(),
        };
        let unit = Gb32960RealtimeUnit::parse(&data.encode()).unwrap();
        assert!(unit.section(SEC_VEHICLE).is_some());
        assert!(unit.section(SEC_MOTOR).is_none());
        assert_eq!(unit.vehicle().unwrap(), vehicle);
    }

    #[test]
    fn vehicle_section_maps_to_realtime_data() {
        let data = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 2,
            run_mode: 3,
            speed_kmh: 56.3,
            odometer_km: 1234.5,
            total_voltage_v: 350.2,
            total_current_a: -12.5,
            soc_percent: 78,
            dc_dc_status: 1,
            gear: 4,
            insulation_kohm: 500,
            accel_pedal_percent: 20,
            brake_pedal_percent: 0,
            tail: vec![0xAA],
        };
        let section = Gb32960Section {
            data_type: SEC_VEHICLE,
            data: data.encode(),
        };
        assert_eq!(section.vehicle_data().unwrap(), data);
        let wrong = Gb32960Section {
            data_type: SEC_MOTOR,
            data: data.encode(),
        };
        assert_eq!(wrong.vehicle_data(), Err(Gb32960Error::DataUnit));
    }

    #[test]
    fn realtime_sections_roundtrip_and_reject_truncated() {
        let sections = vec![
            Gb32960Section {
                data_type: SEC_VEHICLE,
                data: vec![1, 2, 3],
            },
            Gb32960Section {
                data_type: SEC_LOCATION,
                data: vec![4; 20],
            },
            Gb32960Section {
                data_type: SEC_ALARM,
                data: vec![],
            },
        ];
        let mut body = Vec::new();
        for section in &sections {
            body.extend_from_slice(&section.encode());
        }
        assert_eq!(Gb32960Section::decode_all(&body).unwrap(), sections);
        assert_eq!(
            Gb32960Section::decode_all(&[1, 0]),
            Err(Gb32960Error::DataUnit)
        );
        assert_eq!(
            Gb32960Section::decode_all(&[1, 0, 4, 0]),
            Err(Gb32960Error::DataUnit)
        );
    }

    #[test]
    fn heartbeat_frame_has_empty_data_unit() {
        let frame = Gb32960Frame {
            command: GB32960_HEARTBEAT,
            response: 0,
            vin: "LSVAA123456789012".to_string(),
            encrypt: 0,
            data: vec![],
        };
        let raw = frame.encode().unwrap();
        let decoded = Gb32960Frame::decode(&raw).unwrap();
        assert_eq!(decoded.command, GB32960_HEARTBEAT);
        assert!(decoded.data.is_empty());
    }

    #[test]
    fn logout_body_roundtrip() {
        let logout = Gb32960Logout {
            time: [0x26, 0x11, 0x19, 0x10, 0x11, 0x12],
            serial: 7,
        };
        assert_eq!(Gb32960Logout::decode(&logout.encode()).unwrap(), logout);
        assert_eq!(
            Gb32960Logout::decode(&[0; 7]),
            Err(Gb32960Error::LogoutBody)
        );
    }

    #[test]
    fn rejects_bad_start() {
        let mut raw = Gb32960Frame {
            command: GB32960_LOGIN,
            response: 0xfe,
            vin: "LSVAA2A41NX000001".to_string(),
            encrypt: 1,
            data: vec![],
        }
        .encode()
        .unwrap();
        raw[0] = 0;
        assert_eq!(Gb32960Frame::decode(&raw), Err(Gb32960Error::Start));
    }

    #[test]
    fn frame_uses_2025_start_and_checksum_but_accepts_2016() {
        let frame = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0xFE,
            vin: "LSVAA123456789012".into(),
            encrypt: 1,
            data: vec![0; 20],
        };
        let raw = frame.encode().unwrap();
        assert_eq!(&raw[..2], &GB32960_START);
        assert_eq!(raw.len(), 45);
        let legacy = {
            let mut legacy = raw.clone();
            legacy[..2].copy_from_slice(&GB32960_START_2016);
            legacy.pop();
            legacy
        };
        assert_eq!(Gb32960Frame::decode(&legacy).unwrap(), frame);
        let mut corrupt = raw.clone();
        corrupt[2] ^= 0xFF;
        assert_eq!(Gb32960Frame::decode(&corrupt), Err(Gb32960Error::Checksum));
    }

    #[test]
    fn key_exchange_matches_2025_table_31() {
        let exchange = Gb32960KeyExchange {
            key_type: 3,
            key: vec![1, 2, 3, 4],
            enable_time: [26, 1, 1, 0, 0, 0],
            expire_time: [26, 12, 31, 0, 0, 0],
        };
        let body = exchange.encode().unwrap();
        assert_eq!(Gb32960KeyExchange::decode(&body).unwrap(), exchange);
        assert!(Gb32960KeyExchange::decode(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn alarm_level_is_version_aware() {
        assert!(Gb32960Version::V2016.validate_alarm_level(3).is_ok());
        assert!(Gb32960Version::V2016.validate_alarm_level(4).is_err());
        assert!(Gb32960Version::V2025.validate_alarm_level(4).is_ok());
        assert!(Gb32960Version::V2025.validate_alarm_level(5).is_err());
    }

    #[test]
    fn general_alarm_level_list_n5_roundtrips() {
        let alarm = Gb32960AlarmData {
            level: 4,
            flags: 1 << 23,
            battery_faults: vec![],
            motor_faults: vec![],
            engine_faults: vec![],
            other_faults: vec![],
            general_alarms: vec![(23, 4), (24, 2)],
        };
        let body = alarm.encode().unwrap();
        assert_eq!(Gb32960AlarmData::decode(&body).unwrap(), alarm);
        let legacy = alarm.encode_version(Gb32960Version::V2016).unwrap();
        assert_eq!(
            Gb32960AlarmData::decode(&legacy).unwrap().general_alarms,
            vec![]
        );
    }

    #[test]
    fn section_ids_are_version_specific() {
        use Gb32960SectionKind::*;
        assert_eq!(Alarm.id(Gb32960Version::V2016), Some(0x07));
        assert_eq!(Alarm.id(Gb32960Version::V2025), Some(0x06));
        assert_eq!(BatteryVoltage.id(Gb32960Version::V2025), Some(0x07));
        assert_eq!(BatteryTemperature.id(Gb32960Version::V2025), Some(0x08));
        assert_eq!(FuelCellStack.id(Gb32960Version::V2016), None);
        assert_eq!(FuelCellStack.id(Gb32960Version::V2025), Some(0x30));
        assert_eq!(SuperCapacitor.id(Gb32960Version::V2025), Some(0x31));
        assert_eq!(SuperCapacitorExtreme.id(Gb32960Version::V2025), Some(0x32));
        assert_eq!(Vehicle.id(Gb32960Version::V2025), Some(0x01));
    }

    #[test]
    fn battery_2025_voltage_and_temperature_roundtrip() {
        let voltage = Gb32960BatteryVoltage2025 {
            packs: vec![Gb32960BatteryVoltage2025Pack {
                pack_no: 1,
                pack_voltage: 3500,
                pack_current: 3000,
                unit_voltages: vec![3200, 3210, 3190],
            }],
        };
        let body = voltage.encode().unwrap();
        assert_eq!(Gb32960BatteryVoltage2025::decode(&body).unwrap(), voltage);
        assert!(Gb32960BatteryVoltage2025::decode(&body[..body.len() - 1]).is_err());

        let temperature = Gb32960BatteryTemp2025 {
            packs: vec![Gb32960BatteryTemp2025Pack {
                pack_no: 2,
                temperatures: vec![40, 41, 39],
            }],
        };
        let body = temperature.encode().unwrap();
        assert_eq!(Gb32960BatteryTemp2025::decode(&body).unwrap(), temperature);
    }

    #[test]
    fn fuel_cell_and_supercapacitor_2025_roundtrip() {
        let stacks = Gb32960FuelCellStacks {
            stacks: vec![Gb32960FuelCellStack {
                no: 1,
                voltage: 1000,
                current: 200,
                hydrogen_inlet_pressure: 1500,
                air_inlet_pressure: 1200,
                air_inlet_temp: 60,
                coolant_temps: vec![70, 71],
            }],
        };
        let body = stacks.encode().unwrap();
        assert_eq!(Gb32960FuelCellStacks::decode(&body).unwrap(), stacks);
        assert!(Gb32960FuelCellStacks::decode(&body[..body.len() - 1]).is_err());

        let cap = Gb32960SuperCapacitor {
            system_no: 2,
            total_voltage: 5000,
            total_current: 3000,
            cell_voltages: vec![2400, 2450],
            temperatures: vec![30, 31],
        };
        let body = cap.encode().unwrap();
        assert_eq!(Gb32960SuperCapacitor::decode(&body).unwrap(), cap);

        let extreme = Gb32960SuperCapacitorExtreme {
            max_voltage_system: 2,
            max_voltage_cell: 1,
            max_voltage: 2450,
            min_voltage_system: 2,
            min_voltage_cell: 2,
            min_voltage: 2400,
            max_temp_system: 2,
            max_temp_probe: 1,
            max_temp: 31,
            min_temp_system: 2,
            min_temp_probe: 2,
            min_temp: 30,
        };
        let body = extreme.encode();
        assert_eq!(body.len(), 18);
        assert_eq!(
            Gb32960SuperCapacitorExtreme::decode(&body).unwrap(),
            extreme
        );
    }

    #[test]
    fn login_2025_bms_and_pack_codes_roundtrip() {
        let login = Gb32960Login2025 {
            time: [26, 11, 22, 13, 0, 0],
            serial: 7,
            iccid: "89860012345678901234".into(),
            bms_pack_counts: vec![2, 1],
            pack_codes: vec![
                "PACK0000000000000000001".into(),
                "PACK0000000000000000002".into(),
                "PACK0000000000000000003".into(),
            ],
        };
        let body = login.encode().unwrap();
        assert_eq!(Gb32960Login2025::decode(&body).unwrap(), login);
        assert!(Gb32960Login2025::decode(&body[..body.len() - 1]).is_err());
    }

    #[test]
    fn signature_activation_and_custom_data_roundtrip() {
        let signature = Gb32960Signature {
            sign_type: 2,
            r: vec![1, 2, 3],
            s: vec![4, 5],
        };
        assert_eq!(
            Gb32960Signature::decode(&signature.encode().unwrap()).unwrap(),
            signature
        );

        let activation = Gb32960Activation {
            time: [26, 11, 22, 13, 0, 0],
            chip_id: "CHIP000000000001".into(),
            public_key: vec![9; 32],
            vin: "LSVAA123456789012".into(),
            signature: signature.clone(),
        };
        let body = activation.encode().unwrap();
        assert_eq!(Gb32960Activation::decode(&body).unwrap(), activation);
        assert!(Gb32960Activation::decode(&body[..body.len() - 1]).is_err());

        let ack = Gb32960ActivationAck { status: 1, info: 0 };
        assert_eq!(Gb32960ActivationAck::decode(&ack.encode()).unwrap(), ack);

        let custom = Gb32960CustomData {
            data: vec![1, 2, 3],
        };
        assert_eq!(
            Gb32960CustomData::decode(&custom.encode().unwrap()).unwrap(),
            custom
        );
    }

    #[test]
    fn platform_login_2025_matches_table_29() {
        let login = Gb32960PlatformLogin2025 {
            login_time: [26, 11, 22, 14, 0, 0],
            serial: 3,
            username: "tsp-user".into(),
            password: "secret".into(),
            encryption: 3,
        };
        let body = login.encode().unwrap();
        assert_eq!(body.len(), 41);
        assert_eq!(Gb32960PlatformLogin2025::decode(&body).unwrap(), login);
        let mut bad = body.clone();
        bad[40] = 6;
        assert!(Gb32960PlatformLogin2025::decode(&bad).is_err());
    }

    #[test]
    fn rsa_signature_verification_is_explicit_and_works() {
        use rsa::pkcs1v15::SigningKey;
        use rsa::pkcs8::{EncodePublicKey, LineEnding};
        use rsa::signature::{SignatureEncoding, Signer};
        use rsa::{RsaPrivateKey, RsaPublicKey};
        use sha2::Sha256;

        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = RsaPublicKey::from(&private);
        let pem = public.to_public_key_pem(LineEnding::LF).unwrap();
        let signing = SigningKey::<Sha256>::new(private);
        let data = b"32960 signed payload";
        let signature = Gb32960Signature {
            sign_type: 2,
            r: signing.sign(data).to_vec(),
            s: vec![],
        };
        assert!(signature
            .verify_rsa(data, &pem, Gb32960RsaScheme::Pkcs1v15Sha256)
            .unwrap());
        assert!(!signature
            .verify_rsa(b"tampered", &pem, Gb32960RsaScheme::Pkcs1v15Sha256)
            .unwrap());
    }

    #[test]
    fn rsa_key_exchange_decryption_is_explicit() {
        use rsa::pkcs8::{EncodePrivateKey, LineEnding};
        use rsa::{RsaPrivateKey, RsaPublicKey};
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = RsaPublicKey::from(&private);
        let pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
        let plain = b"0123456789abcdef";
        let cipher = public
            .encrypt(&mut rng, rsa::Pkcs1v15Encrypt, plain)
            .unwrap();
        let exchange = Gb32960KeyExchange {
            key_type: 2,
            key: cipher,
            enable_time: [0; 6],
            expire_time: [0; 6],
        };
        assert_eq!(
            exchange
                .decrypt_rsa_key(&pem, Gb32960RsaDecryptScheme::Pkcs1v15)
                .unwrap(),
            plain
        );
    }

    #[test]
    fn aes_128_modes_are_explicit_and_roundtrip() {
        let key = [7u8; 16];
        let iv = [9u8; 16];
        let data = b"32960 aes payload";
        let cipher = encrypt_aes_128(&key, &iv, data, Gb32960AesScheme::CbcPkcs7).unwrap();
        assert_eq!(
            decrypt_aes_128(&key, &iv, &cipher, Gb32960AesScheme::CbcPkcs7).unwrap(),
            data
        );

        let nonce = [1u8; 12];
        let cipher = encrypt_aes_128(&key, &nonce, data, Gb32960AesScheme::Gcm).unwrap();
        assert_eq!(
            decrypt_aes_128(&key, &nonce, &cipher, Gb32960AesScheme::Gcm).unwrap(),
            data
        );
        let mut tampered = cipher.clone();
        tampered[0] ^= 1;
        assert!(decrypt_aes_128(&key, &nonce, &tampered, Gb32960AesScheme::Gcm).is_err());
    }

    #[test]
    fn sm4_cbc_roundtrip_and_tamper_rejection() {
        let key = [3u8; 16];
        let iv = [5u8; 16];
        let data = b"32960 sm4 payload";
        let cipher = encrypt_sm4_cbc(&key, &iv, data).unwrap();
        assert_eq!(decrypt_sm4_cbc(&key, &iv, &cipher).unwrap(), data);
        let mut tampered = cipher.clone();
        tampered[0] ^= 1;
        let _ = decrypt_sm4_cbc(&key, &iv, &tampered);
        assert!(decrypt_sm4_cbc(&key, &iv, &cipher[..15]).is_err());
    }

    #[test]
    fn sm2_signature_verification_works() {
        use sm2::dsa::signature::Signer;
        use sm2::dsa::SigningKey;
        use sm2::elliptic_curve::sec1::ToEncodedPoint;

        let secret = [1u8; 32];
        let distid = "1234567812345678";
        let signing = SigningKey::from_slice(distid, &secret).unwrap();
        let data = b"32960 sm2 payload";
        let sig = signing.sign(data).to_bytes();
        let signature = Gb32960Signature {
            sign_type: 1,
            r: sig[..32].to_vec(),
            s: sig[32..].to_vec(),
        };
        let point = signing.verifying_key().as_affine().to_encoded_point(false);
        assert!(signature
            .verify_sm2(data, point.as_bytes(), distid)
            .unwrap());
        assert!(!signature
            .verify_sm2(b"tampered", point.as_bytes(), distid)
            .unwrap());
    }

    #[test]
    fn ecdsa_p256_signature_verification_works() {
        use p256::ecdsa::signature::Signer;
        use p256::ecdsa::{Signature as P256Signature, SigningKey, VerifyingKey};

        let signing = SigningKey::random(&mut rand::thread_rng());
        let data = b"32960 ecc payload";
        let sig: P256Signature = signing.sign(data);
        let bytes = sig.to_bytes();
        let signature = Gb32960Signature {
            sign_type: 3,
            r: bytes[..32].to_vec(),
            s: bytes[32..].to_vec(),
        };
        let point = VerifyingKey::from(&signing).to_encoded_point(false);
        assert!(signature.verify_ecdsa_p256(data, point.as_bytes()).unwrap());
        assert!(!signature
            .verify_ecdsa_p256(b"tampered", point.as_bytes())
            .unwrap());
    }

    #[test]
    fn sm2_decryption_supports_both_cipher_layouts() {
        use sm2::elliptic_curve::sec1::ToEncodedPoint;
        use sm2::elliptic_curve::{Group, PrimeField};
        use sm2::{ProjectivePoint, Scalar};
        use sm3::{Digest, Sm3};

        use sm2::elliptic_curve::generic_array::GenericArray;
        let d = [1u8; 32];
        let k = [2u8; 32];
        let d_bytes: sm2::FieldBytes = GenericArray::clone_from_slice(&d);
        let k_bytes: sm2::FieldBytes = GenericArray::clone_from_slice(&k);
        let scalar_d = Scalar::from_repr(d_bytes).unwrap();
        let scalar_k = Scalar::from_repr(k_bytes).unwrap();
        let public = (ProjectivePoint::generator() * scalar_d).to_affine();
        let c1 = (ProjectivePoint::generator() * scalar_k).to_affine();
        let shared = (ProjectivePoint::from(public) * scalar_k).to_affine();
        let shared_enc = shared.to_encoded_point(false);
        let x2 = shared_enc.x().unwrap().to_vec();
        let y2 = shared_enc.y().unwrap().to_vec();
        let mut z = x2.clone();
        z.extend_from_slice(&y2);
        let plain = b"sm2 ciphertext payload";
        let mask = sm2_kdf(&z, plain.len()).unwrap();
        let c2: Vec<u8> = plain.iter().zip(mask.iter()).map(|(a, b)| a ^ b).collect();
        let mut hasher = Sm3::new();
        hasher.update(&x2);
        hasher.update(plain);
        hasher.update(&y2);
        let c3 = hasher.finalize();
        let c1_bytes = c1.to_encoded_point(false);
        let mut c1c3c2 = c1_bytes.as_bytes().to_vec();
        c1c3c2.extend_from_slice(&c3);
        c1c3c2.extend_from_slice(&c2);
        assert_eq!(
            decrypt_sm2(&d, &c1c3c2, Gb32960Sm2CipherEncoding::C1C3C2).unwrap(),
            plain
        );

        let mut c1c2c3 = c1_bytes.as_bytes().to_vec();
        c1c2c3.extend_from_slice(&c2);
        c1c2c3.extend_from_slice(&c3);
        assert_eq!(
            decrypt_sm2(&d, &c1c2c3, Gb32960Sm2CipherEncoding::C1C2C3).unwrap(),
            plain
        );
    }

    #[test]
    fn sm2_decryption_matches_gb_t_32918_5_annex_c() {
        fn hex(value: &str) -> Vec<u8> {
            let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
            (0..value.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
                .collect()
        }
        let private: [u8; 32] =
            hex("3945208F7B2144B13F36E38AC6D39F95889393692860B51A42FB81EF4DF7C5B8")
                .try_into()
                .unwrap();
        let c1x = hex("04EBFC718E8D1798620432268E77FEB6415E2EDE0E073C0F4F640ECD2E149A73");
        let c1y = hex("E858F9D81E5430A57B36DAAB8F950A3C64E6EE6A63094D99283AFF767E124DF0");
        let c3 = hex("59983C18F809E262923C53AEC295D30383B54E39D609D160AFCB1908D0BD8766");
        let c2 = hex("21886CA989CA9C7D58087307CA93092D651EFA");
        let mut cipher = vec![0x04];
        cipher.extend_from_slice(&c1x);
        cipher.extend_from_slice(&c1y);
        cipher.extend_from_slice(&c3);
        cipher.extend_from_slice(&c2);
        assert_eq!(
            decrypt_sm2(&private, &cipher, Gb32960Sm2CipherEncoding::C1C3C2).unwrap(),
            b"encryption standard"
        );
    }

    #[test]
    fn sm2_signature_matches_gb_t_32918_2_example() {
        fn hex(value: &str) -> Vec<u8> {
            let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
            (0..value.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
                .collect()
        }
        let x = hex("09F9DF311E5421A150DD7D161E4BC5C672179FAD1833FC076BB08FF356F35020");
        let y = hex("CCEA490CE26775A52DC6EA718CC1AA600AED05FBF35E084A6632F6072DA9AD13");
        let mut public = vec![0x04];
        public.extend_from_slice(&x);
        public.extend_from_slice(&y);
        let signature = Gb32960Signature {
            sign_type: 1,
            r: hex("F5A03B0648D2C4630EEAC513E1BB81A15944DA3827D5B74143AC7EACEEE720B3"),
            s: hex("B1B6AA29DF212FD8763182BC0D421CA1BB9038FD1F7F42D4840B69C485BBC1AA"),
        };
        assert!(signature
            .verify_sm2(b"message digest", &public, "1234567812345678")
            .unwrap());
    }

    #[test]
    fn aes_and_sm4_match_standard_vectors() {
        fn hex(value: &str) -> Vec<u8> {
            let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
            (0..value.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
                .collect()
        }
        // NIST SP 800-38D, AES-128-GCM test case 2.
        let key = [0u8; 16];
        let nonce = [0u8; 12];
        let plain = [0u8; 16];
        let out = encrypt_aes_128(&key, &nonce, &plain, Gb32960AesScheme::Gcm).unwrap();
        let expected = hex("0388dace60b6a392f328c2b971b2fe78ab6e47d42cec13bdf53a67b21257bddf");
        assert_eq!(out, expected);
        assert_eq!(
            decrypt_aes_128(&key, &nonce, &out, Gb32960AesScheme::Gcm).unwrap(),
            plain
        );

        // NIST SP 800-38A F.2.1, first CBC block (PKCS#7 adds a second block).
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex("000102030405060708090a0b0c0d0e0f");
        let plain = hex("6bc1bee22e409f96e93d7e117393172a");
        let out = encrypt_aes_128(&key, &iv, &plain, Gb32960AesScheme::CbcPkcs7).unwrap();
        assert_eq!(
            &out[..16],
            hex("7649abac8119b246cee98e9b12e9197d").as_slice()
        );
        assert_eq!(
            decrypt_aes_128(&key, &iv, &out, Gb32960AesScheme::CbcPkcs7).unwrap(),
            plain
        );

        // GB/T 32907-2016 SM4 example: CBC with zero IV makes block 1 equal ECB.
        let key = hex("0123456789abcdeffedcba9876543210");
        let plain = hex("0123456789abcdeffedcba9876543210");
        let iv = [0u8; 16];
        let out = encrypt_sm4_cbc(&key, &iv, &plain).unwrap();
        assert_eq!(
            &out[..16],
            hex("681edf34d206965e86b3e94f536e4246").as_slice()
        );
        assert_eq!(decrypt_sm4_cbc(&key, &iv, &out).unwrap(), plain);
    }

    #[test]
    fn ecdsa_p256_matches_rfc_6979_vector() {
        fn hex(value: &str) -> Vec<u8> {
            let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
            (0..value.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
                .collect()
        }
        let mut public = vec![0x04];
        public.extend_from_slice(&hex(
            "60FED4BA255A9D31C961EB74C6356D68C049B8923B61FA6CE669622E60F29FB6",
        ));
        public.extend_from_slice(&hex(
            "7903FE1008B8BC99A41AE9E95628BC64F2F1B20C2D7E9F5177A3C294D4462299",
        ));
        let signature = Gb32960Signature {
            sign_type: 3,
            r: hex("EFD48B2AACB6A8FD1140DD9CD45E81D69D2C877B56AAF991C34D0EA84EAF3716"),
            s: hex("F7CB1C942D657C41D436C7A1B6E29F65F3E900DBB9AFF4064DC4AB2F843ACDA8"),
        };
        assert!(signature.verify_ecdsa_p256(b"sample", &public).unwrap());
    }

    #[test]
    fn sm3_matches_gb_t_32905_vector() {
        let digest = sm3_hash(b"abc");
        let expected: [u8; 32] = [
            0x66, 0xc7, 0xf0, 0xf4, 0x62, 0xee, 0xed, 0xd9, 0xd1, 0xf2, 0xd4, 0x6b, 0xdc, 0x10,
            0xe4, 0xe2, 0x41, 0x67, 0xc4, 0x87, 0x5c, 0xf2, 0xf7, 0xa2, 0x29, 0x7d, 0xa0, 0x2b,
            0x8f, 0x4b, 0xa8, 0xe0,
        ];
        assert_eq!(digest, expected);
    }

    #[test]
    fn high_voltage_monitor_samples_and_reports_in_window() {
        let mut monitor =
            Gb32960HighVoltageMonitor::start(Gb32960HighVoltageTrigger::ChargeEnd, 1000);
        assert!(monitor.tick(1000));
        assert!(!monitor.tick(1000));
        assert!(monitor.tick(1001));
        assert_eq!(monitor.samples, 2);
        assert!(!monitor.report_due(1000 + 3569));
        assert!(monitor.report_due(1000 + 3570));
        assert!(monitor.report_due(1000 + 3600));
        monitor.mark_reported();
        assert!(!monitor.report_due(1000 + 3599));
        assert!(!monitor.tick(1000 + 3601));
    }

    #[test]
    fn cached_keys_verify_without_reparsing() {
        use rsa::pkcs1v15::SigningKey;
        use rsa::pkcs8::{DecodePublicKey, EncodePublicKey, LineEnding};
        use rsa::signature::{SignatureEncoding, Signer};
        use rsa::{RsaPrivateKey, RsaPublicKey};
        use sha2::Sha256;

        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = RsaPublicKey::from(&private);
        let pem = public.to_public_key_pem(LineEnding::LF).unwrap();
        let cached = RsaPublicKey::from_public_key_pem(&pem).unwrap();
        let signing = SigningKey::<Sha256>::new(private);
        let data = b"cached rsa payload";
        let signature = Gb32960Signature {
            sign_type: 2,
            r: signing.sign(data).to_vec(),
            s: vec![],
        };
        assert!(signature
            .verify_rsa_with_key(data, &cached, Gb32960RsaScheme::Pkcs1v15Sha256)
            .unwrap());
        assert!(signature
            .verify_rsa(data, &pem, Gb32960RsaScheme::Pkcs1v15Sha256)
            .unwrap());

        use sm2::dsa::SigningKey as Sm2SigningKey;
        let sm2_signing = Sm2SigningKey::from_slice("1234567812345678", &[1u8; 32]).unwrap();
        let sm2_key = sm2_signing.verifying_key().clone();
        let sig = sm2_signing.sign(data).to_bytes();
        let sm2_signature = Gb32960Signature {
            sign_type: 1,
            r: sig[..32].to_vec(),
            s: sig[32..].to_vec(),
        };
        assert!(sm2_signature.verify_sm2_with_key(data, &sm2_key).unwrap());
    }

    #[test]
    fn encrypted_frame_decrypts_after_checksum() {
        let section = Gb32960Section {
            data_type: SEC_VEHICLE,
            data: vec![0u8; 20],
        };
        let plain = section.encode();
        let key = [0u8; 16];
        let nonce = [0u8; 12];
        let cipher = encrypt_aes_128(&key, &nonce, &plain, Gb32960AesScheme::Gcm).unwrap();
        let frame = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0xFE,
            vin: "LSVAA123456789012".into(),
            encrypt: 3,
            data: cipher,
        };
        let mut raw = frame.encode().unwrap();
        let decoded = Gb32960Frame::decode_encrypted(&raw, |method, data| {
            assert_eq!(method, 3);
            decrypt_aes_128(&key, &nonce, data, Gb32960AesScheme::Gcm)
        })
        .unwrap();
        assert_eq!(decoded.data, plain);
        assert_eq!(Gb32960Frame::decode(&raw), Err(Gb32960Error::Encrypted));
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        let mut called = false;
        assert_eq!(
            Gb32960Frame::decode_encrypted(&raw, |_, _| {
                called = true;
                Err(Gb32960Error::Crypto)
            }),
            Err(Gb32960Error::Checksum)
        );
        assert!(!called);
    }

    #[test]
    fn stream_decoder_handles_split_and_coalesced_frames() {
        let section = Gb32960Section {
            data_type: SEC_VEHICLE,
            data: vec![0u8; 20],
        };
        let frame = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0xFE,
            vin: "LSVAA123456789012".into(),
            encrypt: 1,
            data: section.encode(),
        }
        .encode()
        .unwrap();
        let mut decoder = Gb32960StreamDecoder::new();
        decoder.feed(&[0x00, 0x01]);
        decoder.feed(&frame[..10]);
        assert!(decoder.next_frame().is_none());
        decoder.feed(&frame[10..]);
        assert_eq!(decoder.next_frame(), Some(frame.clone()));
        decoder.feed(&frame);
        decoder.feed(&frame);
        assert_eq!(decoder.next_frame(), Some(frame.clone()));
        assert_eq!(decoder.next_frame(), Some(frame));
        assert_eq!(decoder.next_frame(), None);
    }

    #[test]
    fn clock_body_roundtrips_and_rejects_bad_bcd() {
        let clock = Gb32960Clock {
            time: [0x26, 0x11, 0x23, 0x12, 0x30, 0x00],
        };
        assert_eq!(
            Gb32960Clock::decode(&clock.encode().unwrap()).unwrap(),
            clock
        );
        assert!(Gb32960Clock::decode(&[0x26, 0x11, 0x23, 0x12, 0x3A, 0x00]).is_err());
        assert!(Gb32960Clock::decode(&[0x26]).is_err());
    }

    #[test]
    fn unit_exposes_typed_sections_and_rejects_duplicates() {
        let vehicle = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 1,
            run_mode: 1,
            speed_kmh: 1.0,
            odometer_km: 1.0,
            total_voltage_v: 1.0,
            total_current_a: 1.0,
            soc_percent: 50,
            dc_dc_status: 1,
            gear: 1,
            insulation_kohm: 1,
            accel_pedal_percent: 1,
            brake_pedal_percent: 0,
            tail: vec![],
        };
        let location = Gb32960LocationData {
            status: 1,
            lon: 121.0,
            lat: 31.0,
            altitude_m: 1,
        };
        let unit = Gb32960RealtimeUnit::parse(
            &[
                Gb32960Section {
                    data_type: SEC_VEHICLE,
                    data: vehicle.encode(),
                }
                .encode(),
                Gb32960Section {
                    data_type: SEC_LOCATION,
                    data: location.encode().unwrap(),
                }
                .encode(),
            ]
            .concat(),
        )
        .unwrap();
        assert_eq!(unit.location().unwrap(), location);
        assert!(unit.validate_unique_sections().is_ok());
        let duplicate = Gb32960RealtimeUnit::parse(
            &[
                Gb32960Section {
                    data_type: SEC_VEHICLE,
                    data: vehicle.encode(),
                }
                .encode(),
                Gb32960Section {
                    data_type: SEC_VEHICLE,
                    data: vehicle.encode(),
                }
                .encode(),
            ]
            .concat(),
        )
        .unwrap();
        assert_eq!(
            duplicate.validate_unique_sections(),
            Err(Gb32960Error::DataUnit)
        );
    }

    #[test]
    fn unit_validates_version_order_and_signed_sections() {
        let vehicle = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 1,
            run_mode: 1,
            speed_kmh: 1.0,
            odometer_km: 1.0,
            total_voltage_v: 1.0,
            total_current_a: 1.0,
            soc_percent: 50,
            dc_dc_status: 1,
            gear: 1,
            insulation_kohm: 1,
            accel_pedal_percent: 1,
            brake_pedal_percent: 0,
            tail: vec![],
        };
        let location = Gb32960LocationData {
            status: 1,
            lon: 121.0,
            lat: 31.0,
            altitude_m: 1,
        };
        let unit = Gb32960RealtimeUnit::parse(
            &[
                Gb32960Section {
                    data_type: SEC_VEHICLE,
                    data: vehicle.encode(),
                }
                .encode(),
                Gb32960Section {
                    data_type: SEC_LOCATION,
                    data: location.encode().unwrap(),
                }
                .encode(),
            ]
            .concat(),
        )
        .unwrap();
        assert!(unit.validate_sections(Gb32960Version::V2025).is_ok());
        let reversed = Gb32960RealtimeUnit::parse(
            &[
                Gb32960Section {
                    data_type: SEC_LOCATION,
                    data: location.encode().unwrap(),
                }
                .encode(),
                Gb32960Section {
                    data_type: SEC_VEHICLE,
                    data: vehicle.encode(),
                }
                .encode(),
            ]
            .concat(),
        )
        .unwrap();
        assert_eq!(
            reversed.validate_sections(Gb32960Version::V2025),
            Err(Gb32960Error::DataUnit)
        );
    }

    #[test]
    fn engine_and_signal_sections_roundtrip() {
        let engine = Gb32960EngineData {
            status: 1,
            crank_speed_rpm: 1500,
            fuel_rate_raw: 250,
        };
        assert_eq!(Gb32960EngineData::decode(&engine.encode()).unwrap(), engine);
        let signal = Gb32960SignalState { status: 0x05 };
        assert_eq!(
            Gb32960SignalState::decode(&signal.encode()).unwrap(),
            signal
        );
        assert!(Gb32960EngineData::decode(&[1, 2]).is_err());
    }
}
