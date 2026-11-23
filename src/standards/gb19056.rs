//! GB/T 19056 tachograph data blocks carried inside JT/T 808 0x0700.
//!
//! Only the blocks whose layouts were verified against the published command
//! table are decoded here. Other command words remain opaque bytes.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Gb19056Error {
    #[error("tachograph block is too short")]
    TooShort,
    #[error("tachograph BCD is invalid")]
    Bcd,
    #[error("tachograph ASCII is invalid")]
    Ascii,
    #[error("tachograph checksum mismatch")]
    Checksum,
    #[error("tachograph record block is invalid")]
    RecordBody,
}

/// 2021 table A.5 verification record (SFmt=FCH/FEH).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056VerifyRecord {
    pub mfmt: u8,
    pub final_block: bool,
    pub time: [u8; 6],
    pub target_blocks: u16,
    pub target_bytes: u16,
    pub digest: [u8; 32],
}

impl Gb19056VerifyRecord {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        let mut data = Vec::with_capacity(43);
        data.extend_from_slice(&self.time);
        data.push(0x03);
        data.extend_from_slice(&self.target_blocks.to_be_bytes());
        data.extend_from_slice(&self.target_bytes.to_be_bytes());
        data.extend_from_slice(&self.digest);
        let sfmt = if self.final_block { 0xFE } else { 0xFC };
        Gb19056RecordBlock {
            mfmt: self.mfmt,
            sfmt,
            data,
        }
        .encode()
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        let block = Gb19056RecordBlock::decode(raw)?;
        if block.sfmt != 0xFC && block.sfmt != 0xFE {
            return Err(Gb19056Error::RecordBody);
        }
        if block.data.len() < 43 || block.data[6] != 0x03 {
            return Err(Gb19056Error::RecordBody);
        }
        let d = &block.data;
        Ok(Self {
            mfmt: block.mfmt,
            final_block: block.sfmt == 0xFE,
            time: d[0..6].try_into().unwrap(),
            target_blocks: u16::from_be_bytes([d[7], d[8]]),
            target_bytes: u16::from_be_bytes([d[9], d[10]]),
            digest: d[11..43].try_into().unwrap(),
        })
    }
}

/// SM3(target data + 16-byte Salt), per GB/T 19056-2021 A.2.
pub fn sm3_target_digest(target: &[u8], salt: &[u8; 16]) -> [u8; 32] {
    let mut data = Vec::with_capacity(target.len() + 16);
    data.extend_from_slice(target);
    data.extend_from_slice(salt);
    crate::standards::gb32960::sm3_hash(&data)
}

/// Result of walking a complete 19056 data file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gb19056FileVerification {
    pub data_blocks: usize,
    pub verification_blocks: usize,
    pub final_seen: bool,
}

/// Walk 16-byte-aligned record blocks, verifying each FCH/FEH block's SM3
/// digest over the previous start/verification block plus intervening data.
pub fn verify_file_blocks(
    raw: &[u8],
    salt: &[u8; 16],
) -> Result<Gb19056FileVerification, Gb19056Error> {
    if raw.is_empty() || !raw.len().is_multiple_of(16) {
        return Err(Gb19056Error::RecordBody);
    }
    let mut offset = 0usize;
    let mut segment_start = 0usize;
    let mut data_blocks = 0usize;
    let mut verification_blocks = 0usize;
    let mut final_seen = false;
    while offset < raw.len() {
        if offset + 16 > raw.len() {
            return Err(Gb19056Error::RecordBody);
        }
        let size = u16::from_be_bytes([raw[offset + 4], raw[offset + 5]]) as usize;
        if size == 0 || offset + size * 16 > raw.len() {
            return Err(Gb19056Error::RecordBody);
        }
        let block = Gb19056RecordBlock::decode(&raw[offset..offset + size * 16])?;
        match block.sfmt {
            0xFB => segment_start = offset,
            0xFC | 0xFE => {
                let target = &raw[segment_start..offset];
                let expected = sm3_target_digest(target, salt);
                if block.data.len() < 43 || block.data[11..43] != expected {
                    return Err(Gb19056Error::Checksum);
                }
                verification_blocks += 1;
                segment_start = offset;
                if block.sfmt == 0xFE {
                    final_seen = true;
                }
            }
            _ => data_blocks += 1,
        }
        offset += size * 16;
    }
    Ok(Gb19056FileVerification {
        data_blocks,
        verification_blocks,
        final_seen,
    })
}

/// 2021 table A.13 log record (MFmt=25H).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056LogRecord {
    pub sfmt: u8,
    pub time: [u8; 6],
    /// 35-byte event description defined by SFmt in table A.14.
    pub detail: [u8; 35],
}

impl Gb19056LogRecord {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        let mut data = Vec::with_capacity(41);
        data.extend_from_slice(&self.time);
        data.extend_from_slice(&self.detail);
        Gb19056RecordBlock {
            mfmt: 0x25,
            sfmt: self.sfmt,
            data,
        }
        .encode()
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        let block = Gb19056RecordBlock::decode(raw)?;
        if block.mfmt != 0x25 || block.data.len() < 41 {
            return Err(Gb19056Error::RecordBody);
        }
        Ok(Self {
            sfmt: block.sfmt,
            time: block.data[0..6].try_into().unwrap(),
            detail: block.data[6..41].try_into().unwrap(),
        })
    }
}

/// 2021 table A.12 driver-info record (MFmt=24H, SFmt=01H).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056DriverInfoRecord {
    pub time: [u8; 6],
    pub name: String,
    pub licence: String,
    pub photo: Vec<u8>,
}

impl Gb19056DriverInfoRecord {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        let mut data = Vec::with_capacity(56 + self.photo.len());
        data.extend_from_slice(&self.time);
        data.extend_from_slice(&gb19056_fixed(&self.name, 32)?);
        data.extend_from_slice(&gb19056_fixed(&self.licence, 18)?);
        data.extend_from_slice(&self.photo);
        Gb19056RecordBlock {
            mfmt: 0x24,
            sfmt: 0x01,
            data,
        }
        .encode()
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        let block = Gb19056RecordBlock::decode(raw)?;
        if block.mfmt != 0x24 || block.sfmt != 0x01 || block.data.len() < 56 {
            return Err(Gb19056Error::RecordBody);
        }
        let d = &block.data;
        let photo = &d[56..];
        let len = image_len(photo);
        Ok(Self {
            time: d[0..6].try_into().unwrap(),
            name: gb19056_read(&d[6..38])?,
            licence: gb19056_read(&d[38..56])?,
            photo: photo[..len].to_vec(),
        })
    }
}

fn image_len(data: &[u8]) -> usize {
    if data.len() >= 2 && data[..2] == [0xFF, 0xD8] {
        if let Some(pos) = data.windows(2).rposition(|w| w == [0xFF, 0xD9]) {
            return pos + 2;
        }
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return (u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize + 8)
            .min(data.len());
    }
    if data.len() >= 6 && &data[..2] == b"BM" {
        return (u32::from_le_bytes([data[2], data[3], data[4], data[5]]) as usize).min(data.len());
    }
    data.iter().rposition(|b| *b != 0).map_or(0, |pos| pos + 1)
}

/// 2021 table A.11 overtime-driving record (MFmt=23H, SFmt=01H).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056OvertimeRecord {
    pub licence: String,
    pub start_time: [u8; 6],
    pub start_lon: i32,
    pub start_lat: i32,
    pub start_altitude: i16,
    pub end_time: [u8; 6],
    pub end_lon: i32,
    pub end_lat: i32,
    pub end_altitude: i16,
}

impl Gb19056OvertimeRecord {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        let mut data = Vec::with_capacity(50);
        data.extend_from_slice(&gb19056_fixed(&self.licence, 18)?);
        data.extend_from_slice(&self.start_time);
        data.extend_from_slice(&self.start_lon.to_be_bytes());
        data.extend_from_slice(&self.start_lat.to_be_bytes());
        data.extend_from_slice(&self.start_altitude.to_be_bytes());
        data.extend_from_slice(&self.end_time);
        data.extend_from_slice(&self.end_lon.to_be_bytes());
        data.extend_from_slice(&self.end_lat.to_be_bytes());
        data.extend_from_slice(&self.end_altitude.to_be_bytes());
        Gb19056RecordBlock {
            mfmt: 0x23,
            sfmt: 0x01,
            data,
        }
        .encode()
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        let block = Gb19056RecordBlock::decode(raw)?;
        if block.mfmt != 0x23 || block.sfmt != 0x01 || block.data.len() < 50 {
            return Err(Gb19056Error::RecordBody);
        }
        let d = &block.data;
        Ok(Self {
            licence: gb19056_read(&d[0..18])?,
            start_time: d[18..24].try_into().unwrap(),
            start_lon: i32::from_be_bytes(d[24..28].try_into().unwrap()),
            start_lat: i32::from_be_bytes(d[28..32].try_into().unwrap()),
            start_altitude: i16::from_be_bytes(d[32..34].try_into().unwrap()),
            end_time: d[34..40].try_into().unwrap(),
            end_lon: i32::from_be_bytes(d[40..44].try_into().unwrap()),
            end_lat: i32::from_be_bytes(d[44..48].try_into().unwrap()),
            end_altitude: i16::from_be_bytes(d[48..50].try_into().unwrap()),
        })
    }
}

/// 2021 table A.10 accident-doubt record (MFmt=22H, SFmt=01H).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056AccidentRecord {
    pub time: [u8; 6],
    pub licence: String,
    pub lon: i32,
    pub lat: i32,
    pub altitude: i16,
    /// 200 samples at 0.1 s, newest first.
    pub speeds: Vec<u8>,
    /// 200 switch words aligned with `speeds`.
    pub switches: Vec<u16>,
}

impl Gb19056AccidentRecord {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        if self.speeds.len() != 200 || self.switches.len() != 200 {
            return Err(Gb19056Error::RecordBody);
        }
        let mut data = Vec::with_capacity(634);
        data.extend_from_slice(&self.time);
        data.extend_from_slice(&gb19056_fixed(&self.licence, 18)?);
        data.extend_from_slice(&self.lon.to_be_bytes());
        data.extend_from_slice(&self.lat.to_be_bytes());
        data.extend_from_slice(&self.altitude.to_be_bytes());
        data.extend_from_slice(&self.speeds);
        for switches in &self.switches {
            data.extend_from_slice(&switches.to_be_bytes());
        }
        Gb19056RecordBlock {
            mfmt: 0x22,
            sfmt: 0x01,
            data,
        }
        .encode()
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        let block = Gb19056RecordBlock::decode(raw)?;
        if block.mfmt != 0x22 || block.sfmt != 0x01 || block.data.len() < 634 {
            return Err(Gb19056Error::RecordBody);
        }
        let d = &block.data;
        let mut switches = Vec::with_capacity(200);
        let mut offset = 234;
        for _ in 0..200 {
            switches.push(u16::from_be_bytes([d[offset], d[offset + 1]]));
            offset += 2;
        }
        Ok(Self {
            time: d[0..6].try_into().unwrap(),
            licence: gb19056_read(&d[6..24])?,
            lon: i32::from_be_bytes(d[24..28].try_into().unwrap()),
            lat: i32::from_be_bytes(d[28..32].try_into().unwrap()),
            altitude: i16::from_be_bytes(d[32..34].try_into().unwrap()),
            speeds: d[34..234].to_vec(),
            switches,
        })
    }
}

/// 2021 table A.10 driving-state record (MFmt=21H, SFmt=11H).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056DriveRecord {
    pub time: [u8; 6],
    pub lon: i32,
    pub lat: i32,
    pub altitude: i16,
    pub direction: u8,
    pub speed: u8,
    pub switches: u16,
    pub reference_speed: u8,
    pub status: u8,
}

impl Gb19056DriveRecord {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        let mut data = Vec::with_capacity(22);
        data.extend_from_slice(&self.time);
        data.extend_from_slice(&self.lon.to_be_bytes());
        data.extend_from_slice(&self.lat.to_be_bytes());
        data.extend_from_slice(&self.altitude.to_be_bytes());
        data.push(self.direction);
        data.push(self.speed);
        data.extend_from_slice(&self.switches.to_be_bytes());
        data.push(self.reference_speed);
        data.push(self.status);
        Gb19056RecordBlock {
            mfmt: 0x21,
            sfmt: 0x11,
            data,
        }
        .encode()
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        let block = Gb19056RecordBlock::decode(raw)?;
        if block.mfmt != 0x21 || block.sfmt != 0x11 || block.data.len() < 22 {
            return Err(Gb19056Error::RecordBody);
        }
        let d = &block.data;
        Ok(Self {
            time: d[0..6].try_into().unwrap(),
            lon: i32::from_be_bytes(d[6..10].try_into().unwrap()),
            lat: i32::from_be_bytes(d[10..14].try_into().unwrap()),
            altitude: i16::from_be_bytes(d[14..16].try_into().unwrap()),
            direction: d[16],
            speed: d[17],
            switches: u16::from_be_bytes([d[18], d[19]]),
            reference_speed: d[20],
            status: d[21],
        })
    }
}

/// 2021 table A.3 start record (SFmt=FBH).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056StartRecord {
    pub mfmt: u8,
    pub standard_year: [u8; 2],
    pub file_name: String,
    pub file_time: [u8; 6],
    pub file_length: u32,
    pub file_type: String,
    pub unique_id: [u8; 35],
    pub plate_no: String,
    pub plate_class: String,
    pub vin: String,
    pub e_mark: [u8; 6],
    pub pulse: u16,
    pub install_time: [u8; 6],
    pub digest: [u8; 32],
}

impl Gb19056StartRecord {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        let mut data = Vec::with_capacity(202);
        data.extend_from_slice(&self.standard_year);
        data.extend_from_slice(&gb19056_fixed(&self.file_name, 32)?);
        data.extend_from_slice(&self.file_time);
        data.extend_from_slice(&self.file_length.to_be_bytes());
        data.extend_from_slice(&gb19056_fixed(&self.file_type, 20)?);
        data.extend_from_slice(&[0u8; 10]);
        data.extend_from_slice(&self.unique_id);
        data.extend_from_slice(&gb19056_fixed(&self.plate_no, 14)?);
        data.extend_from_slice(&gb19056_fixed(&self.plate_class, 16)?);
        data.extend_from_slice(&gb19056_fixed(&self.vin, 17)?);
        data.extend_from_slice(&self.e_mark);
        data.extend_from_slice(&self.pulse.to_be_bytes());
        data.extend_from_slice(&self.install_time);
        data.extend_from_slice(&self.digest);
        if data.len() != 202 {
            return Err(Gb19056Error::RecordBody);
        }
        Gb19056RecordBlock {
            mfmt: self.mfmt,
            sfmt: 0xFB,
            data,
        }
        .encode()
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        let block = Gb19056RecordBlock::decode(raw)?;
        if block.sfmt != 0xFB || block.data.len() < 202 {
            return Err(Gb19056Error::RecordBody);
        }
        let d = &block.data;
        Ok(Self {
            mfmt: block.mfmt,
            standard_year: d[0..2].try_into().unwrap(),
            file_name: gb19056_read(&d[2..34])?,
            file_time: d[34..40].try_into().unwrap(),
            file_length: u32::from_be_bytes(d[40..44].try_into().unwrap()),
            file_type: gb19056_read(&d[44..64])?,
            unique_id: d[74..109].try_into().unwrap(),
            plate_no: gb19056_read(&d[109..123])?,
            plate_class: gb19056_read(&d[123..139])?,
            vin: gb19056_read(&d[139..156])?,
            e_mark: d[156..162].try_into().unwrap(),
            pulse: u16::from_be_bytes([d[162], d[163]]),
            install_time: d[164..170].try_into().unwrap(),
            digest: d[170..202].try_into().unwrap(),
        })
    }
}

fn gb19056_fixed(value: &str, width: usize) -> Result<Vec<u8>, Gb19056Error> {
    let (bytes, _, bad) = encoding_rs::GBK.encode(value);
    if bad || bytes.len() > width {
        return Err(Gb19056Error::RecordBody);
    }
    let mut out = vec![0u8; width];
    out[..bytes.len()].copy_from_slice(&bytes);
    Ok(out)
}

fn gb19056_read(field: &[u8]) -> Result<String, Gb19056Error> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    let (text, _, bad) = encoding_rs::GBK.decode(&field[..end]);
    if bad {
        return Err(Gb19056Error::RecordBody);
    }
    Ok(text.into_owned())
}

pub const GB19056_RECORD_HEADER: [u8; 2] = [0x55, 0x76];

/// 2021 表 A.2 record block: 16-byte aligned, 55 76 + MFmt/SFmt/Size + data + fill + XOR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056RecordBlock {
    pub mfmt: u8,
    pub sfmt: u8,
    pub data: Vec<u8>,
}

impl Gb19056RecordBlock {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        if !(0x21..=0x25).contains(&self.mfmt) {
            return Err(Gb19056Error::RecordBody);
        }
        let mut total = 6 + self.data.len() + 1;
        if !total.is_multiple_of(16) {
            total += 16 - total % 16;
        }
        if total / 16 > u16::MAX as usize {
            return Err(Gb19056Error::RecordBody);
        }
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&GB19056_RECORD_HEADER);
        out.push(self.mfmt);
        out.push(self.sfmt);
        out.extend_from_slice(&((total / 16) as u16).to_be_bytes());
        out.extend_from_slice(&self.data);
        out.resize(total - 1, 0);
        let checksum = out.iter().fold(0u8, |acc, byte| acc ^ byte);
        out.push(checksum);
        Ok(out)
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        if raw.len() < 16 || raw[..2] != GB19056_RECORD_HEADER || !raw.len().is_multiple_of(16) {
            return Err(Gb19056Error::RecordBody);
        }
        let mfmt = raw[2];
        if !(0x21..=0x25).contains(&mfmt) {
            return Err(Gb19056Error::RecordBody);
        }
        let size = u16::from_be_bytes([raw[4], raw[5]]) as usize;
        if size != raw.len() / 16 {
            return Err(Gb19056Error::RecordBody);
        }
        let checksum = raw[..raw.len() - 1]
            .iter()
            .fold(0u8, |acc, byte| acc ^ byte);
        if checksum != raw[raw.len() - 1] {
            return Err(Gb19056Error::Checksum);
        }
        Ok(Self {
            mfmt,
            sfmt: raw[3],
            data: raw[6..raw.len() - 1].to_vec(),
        })
    }
}

pub const GB19056_CMD_HEADER: [u8; 2] = [0xAA, 0x75];
pub const GB19056_ACK_HEADER: [u8; 2] = [0x55, 0x7A];

/// Command/ACK frame from GB/T 19056 appendix A tables A.1/A.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056Frame {
    pub command: u8,
    pub data: Vec<u8>,
}

impl Gb19056Frame {
    pub fn encode(&self) -> Result<Vec<u8>, Gb19056Error> {
        if self.data.len() > u16::MAX as usize {
            return Err(Gb19056Error::TooShort);
        }
        let mut out = Vec::with_capacity(8 + self.data.len());
        out.extend_from_slice(&GB19056_CMD_HEADER);
        out.push(self.command);
        out.extend_from_slice(&(self.data.len() as u16).to_be_bytes());
        out.push(0);
        out.extend_from_slice(&self.data);
        let checksum = out.iter().fold(0u8, |acc, byte| acc ^ byte);
        out.push(checksum);
        Ok(out)
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Gb19056Error> {
        if raw.len() < 8 || raw[..2] != GB19056_CMD_HEADER {
            return Err(Gb19056Error::TooShort);
        }
        let command = raw[2];
        let len = u16::from_be_bytes([raw[3], raw[4]]) as usize;
        if raw[5] != 0 || raw.len() != 6 + len + 1 {
            return Err(Gb19056Error::TooShort);
        }
        let checksum = raw
            .iter()
            .take(raw.len() - 1)
            .fold(0u8, |acc, byte| acc ^ byte);
        if checksum != raw[raw.len() - 1] {
            return Err(Gb19056Error::Checksum);
        }
        Ok(Self {
            command,
            data: raw[6..6 + len].to_vec(),
        })
    }

    /// Encode an ACK frame using the 0x55 0x7A header.
    pub fn encode_ack(&self) -> Result<Vec<u8>, Gb19056Error> {
        let mut out = self.encode()?;
        out[..2].copy_from_slice(&GB19056_ACK_HEADER);
        let checksum = out[..out.len() - 1]
            .iter()
            .fold(0u8, |acc, byte| acc ^ byte);
        let last = out.len() - 1;
        out[last] = checksum;
        Ok(out)
    }
}

/// 00H: executing standard version, BCD year + revision byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gb19056Version {
    pub year: u8,
    pub revision: u8,
}

pub fn decode_version(data: &[u8]) -> Result<Gb19056Version, Gb19056Error> {
    if data.len() < 2 {
        return Err(Gb19056Error::TooShort);
    }
    let high = data[0] >> 4;
    let low = data[0] & 0x0F;
    if high > 9 || low > 9 {
        return Err(Gb19056Error::Bcd);
    }
    Ok(Gb19056Version {
        year: high * 10 + low,
        revision: data[1],
    })
}

/// 01H: current driver licence number, 18 ASCII bytes, NUL padded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gb19056Driver {
    pub licence: String,
}

pub fn decode_driver(data: &[u8]) -> Result<Gb19056Driver, Gb19056Error> {
    if data.len() < 18 {
        return Err(Gb19056Error::TooShort);
    }
    let end = data[..18].iter().position(|b| *b == 0).unwrap_or(18);
    if !data[..end]
        .iter()
        .all(|b| b.is_ascii_graphic() || *b == b' ')
    {
        return Err(Gb19056Error::Ascii);
    }
    Ok(Gb19056Driver {
        licence: String::from_utf8_lossy(&data[..end]).trim().to_string(),
    })
}

/// 02H: recorder real-time clock, BCD YY-MM-DD-hh-mm-ss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gb19056Clock {
    pub time: [u8; 6],
}

pub fn decode_clock(data: &[u8]) -> Result<Gb19056Clock, Gb19056Error> {
    if data.len() < 6 {
        return Err(Gb19056Error::TooShort);
    }
    for byte in &data[..6] {
        if byte >> 4 > 9 || byte & 0x0F > 9 {
            return Err(Gb19056Error::Bcd);
        }
    }
    Ok(Gb19056Clock {
        time: data[..6].try_into().unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_driver_and_clock_blocks() {
        assert_eq!(
            decode_version(&[0x21, 0x00]).unwrap(),
            Gb19056Version {
                year: 21,
                revision: 0
            }
        );
        let mut licence = *b"110101199001011234\0\0\0";
        assert_eq!(
            decode_driver(&licence).unwrap().licence,
            "110101199001011234"
        );
        licence[0] = 0xFF;
        assert!(decode_driver(&licence).is_err());
        assert_eq!(
            decode_clock(&[0x26, 0x11, 0x23, 0x12, 0x34, 0x56])
                .unwrap()
                .time,
            [0x26, 0x11, 0x23, 0x12, 0x34, 0x56]
        );
        assert!(decode_clock(&[0x2A, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn command_frame_roundtrip_and_xor() {
        let frame = Gb19056Frame {
            command: 0x01,
            data: vec![1, 2, 3],
        };
        let raw = frame.encode().unwrap();
        assert_eq!(&raw[..2], &[0xAA, 0x75]);
        assert_eq!(Gb19056Frame::decode(&raw).unwrap(), frame);
        let mut bad = raw.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0x01;
        assert!(Gb19056Frame::decode(&bad).is_err());
        let ack = frame.encode_ack().unwrap();
        assert_eq!(&ack[..2], &[0x55, 0x7A]);
    }

    #[test]
    fn record_block_is_16_byte_aligned() {
        let block = Gb19056RecordBlock {
            mfmt: 0x21,
            sfmt: 0x01,
            data: vec![1, 2, 3],
        };
        let raw = block.encode().unwrap();
        assert_eq!(raw.len() % 16, 0);
        assert_eq!(&raw[..2], &[0x55, 0x76]);
        assert_eq!(
            u16::from_be_bytes([raw[4], raw[5]]) as usize,
            raw.len() / 16
        );
        assert_eq!(Gb19056RecordBlock::decode(&raw).unwrap().mfmt, 0x21);
        assert_eq!(Gb19056RecordBlock::decode(&raw).unwrap().sfmt, 0x01);
        let mut bad = raw.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        assert_eq!(
            Gb19056RecordBlock::decode(&bad),
            Err(Gb19056Error::Checksum)
        );
        assert!(Gb19056RecordBlock {
            mfmt: 0x20,
            sfmt: 0,
            data: vec![]
        }
        .encode()
        .is_err());
    }

    #[test]
    fn start_record_roundtrip() {
        let record = Gb19056StartRecord {
            mfmt: 0x21,
            standard_year: [0x20, 0x21],
            file_name: "GBT19056_2100.VDR".into(),
            file_time: [0x26, 0x01, 0x01, 0x00, 0x00, 0x00],
            file_length: 4096,
            file_type: "行驶状态记录".into(),
            unique_id: [1u8; 35],
            plate_no: "沪A12345".into(),
            plate_class: "大型汽车号牌".into(),
            vin: "LSVAA123456789012".into(),
            e_mark: [0u8; 6],
            pulse: 8000,
            install_time: [0x26, 0x01, 0x01, 0x00, 0x00, 0x00],
            digest: [9u8; 32],
        };
        let raw = record.encode().unwrap();
        assert_eq!(raw.len(), 224);
        assert_eq!(Gb19056StartRecord::decode(&raw).unwrap(), record);
    }

    #[test]
    fn verification_record_and_sm3_digest() {
        let digest = sm3_target_digest(b"target", &[7u8; 16]);
        assert_eq!(
            digest,
            crate::standards::gb32960::sm3_hash(&[b"target".as_slice(), &[7u8; 16]].concat())
        );
        let record = Gb19056VerifyRecord {
            mfmt: 0x21,
            final_block: true,
            time: [0x26, 0x01, 0x01, 0x00, 0x00, 0x00],
            target_blocks: 3,
            target_bytes: 720,
            digest,
        };
        let raw = record.encode().unwrap();
        assert_eq!(raw.len(), 64);
        assert_eq!(Gb19056VerifyRecord::decode(&raw).unwrap(), record);
        assert!(Gb19056VerifyRecord::decode(&raw[..32]).is_err());
    }

    #[test]
    fn driving_state_record_roundtrips() {
        let record = Gb19056DriveRecord {
            time: [0x26, 11, 23, 12, 0, 0],
            lon: -121_473_700,
            lat: 31_230_400,
            altitude: 12,
            direction: 90,
            speed: 60,
            switches: 0x8001,
            reference_speed: 61,
            status: 0x04,
        };
        let raw = record.encode().unwrap();
        assert_eq!(raw.len(), 32);
        assert_eq!(Gb19056DriveRecord::decode(&raw).unwrap(), record);
    }

    #[test]
    fn accident_doubt_record_roundtrips() {
        let record = Gb19056AccidentRecord {
            time: [0x26, 11, 23, 12, 0, 0],
            licence: "110101199001011234".into(),
            lon: 121_473_700,
            lat: 31_230_400,
            altitude: 12,
            speeds: (0..200).map(|i| (i % 220) as u8).collect(),
            switches: (0..200).map(|i| i as u16).collect(),
        };
        let raw = record.encode().unwrap();
        assert_eq!(raw.len(), 656);
        assert_eq!(Gb19056AccidentRecord::decode(&raw).unwrap(), record);
        let mut short = record.clone();
        short.speeds.pop();
        assert!(short.encode().is_err());
    }

    #[test]
    fn overtime_record_roundtrips() {
        let record = Gb19056OvertimeRecord {
            licence: "110101199001011234".into(),
            start_time: [0x26, 11, 23, 8, 0, 0],
            start_lon: 121_473_700,
            start_lat: 31_230_400,
            start_altitude: 10,
            end_time: [0x26, 11, 23, 12, 30, 0],
            end_lon: 121_500_000,
            end_lat: 31_250_000,
            end_altitude: -5,
        };
        let raw = record.encode().unwrap();
        assert_eq!(raw.len(), 64);
        assert_eq!(Gb19056OvertimeRecord::decode(&raw).unwrap(), record);
    }

    #[test]
    fn driver_info_record_keeps_photo() {
        let record = Gb19056DriverInfoRecord {
            time: [0x26, 11, 23, 12, 0, 0],
            name: "张三".into(),
            licence: "110101199001011234".into(),
            photo: vec![0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9],
        };
        let raw = record.encode().unwrap();
        assert_eq!(raw.len() % 16, 0);
        assert_eq!(Gb19056DriverInfoRecord::decode(&raw).unwrap(), record);
    }

    #[test]
    fn log_record_roundtrips() {
        let mut detail = [0u8; 35];
        detail[..6].copy_from_slice(b"\x00\x00\xb3\xa1\x00\x00");
        let record = Gb19056LogRecord {
            sfmt: 0x21,
            time: [0x26, 11, 23, 12, 0, 0],
            detail,
        };
        let raw = record.encode().unwrap();
        assert_eq!(raw.len(), 48);
        assert_eq!(Gb19056LogRecord::decode(&raw).unwrap(), record);
    }

    #[test]
    fn file_verification_detects_tampering() {
        let salt = [3u8; 16];
        let start = Gb19056StartRecord {
            mfmt: 0x21,
            standard_year: [0x20, 0x21],
            file_name: "GBT19056_2100.VDR".into(),
            file_time: [0x26, 1, 1, 0, 0, 0],
            file_length: 0,
            file_type: "行驶状态记录".into(),
            unique_id: [1u8; 35],
            plate_no: "沪A12345".into(),
            plate_class: "大型汽车号牌".into(),
            vin: "LSVAA123456789012".into(),
            e_mark: [0; 6],
            pulse: 8000,
            install_time: [0x26, 1, 1, 0, 0, 0],
            digest: [0u8; 32],
        }
        .encode()
        .unwrap();
        let data = Gb19056DriveRecord {
            time: [0x26, 11, 23, 12, 0, 0],
            lon: 1,
            lat: 2,
            altitude: 3,
            direction: 4,
            speed: 5,
            switches: 6,
            reference_speed: 7,
            status: 8,
        }
        .encode()
        .unwrap();
        let target: Vec<u8> = [start.clone(), data.clone()].concat();
        let digest = sm3_target_digest(&target, &salt);
        let verify = Gb19056VerifyRecord {
            mfmt: 0x21,
            final_block: true,
            time: [0x26, 11, 23, 12, 0, 5],
            target_blocks: 2,
            target_bytes: target.len() as u16,
            digest,
        }
        .encode()
        .unwrap();
        let file: Vec<u8> = [start, data, verify].concat();
        let outcome = verify_file_blocks(&file, &salt).unwrap();
        assert!(outcome.final_seen);
        assert_eq!(outcome.verification_blocks, 1);
        let mut tampered = file.clone();
        tampered[224 + 6] ^= 1;
        assert_eq!(
            verify_file_blocks(&tampered, &salt),
            Err(Gb19056Error::Checksum)
        );
    }
}
