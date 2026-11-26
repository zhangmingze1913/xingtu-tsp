//! JT/T 1078 media packet envelope candidate.
//! Structure only: codec/frame semantics and regulator interoperability are not
//! implemented and this is not certification evidence.

use std::collections::{BTreeMap, HashMap};

use thiserror::Error;

pub const JT1078_MAGIC: [u8; 4] = [0x30, 0x31, 0x63, 0x64];

pub const MSG_REALTIME_REQUEST: u16 = 0x9101;
pub const MSG_REALTIME_CONTROL: u16 = 0x9102;
pub const MSG_PLAYBACK_REQUEST: u16 = 0x9201;
pub const MSG_PLAYBACK_CONTROL: u16 = 0x9202;
pub const MSG_RESOURCE_LIST_REQUEST: u16 = 0x9205;
pub const MSG_RESOURCE_LIST_UPLOAD: u16 = 0x1205;
pub const MSG_FILE_UPLOAD_COMPLETE: u16 = 0x1206;
pub const MSG_FILE_UPLOAD_COMMAND: u16 = 0x9206;
pub const MSG_FILE_UPLOAD_CONTROL: u16 = 0x9207;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jt1078Signal {
    RealtimeRequest,
    RealtimeControl,
    PlaybackRequest,
    PlaybackControl,
    ResourceListRequest,
    ResourceListUpload,
    FileUploadComplete,
    Unknown(u16),
}

pub fn classify_signal(msg_id: u16) -> Jt1078Signal {
    match msg_id {
        MSG_REALTIME_REQUEST => Jt1078Signal::RealtimeRequest,
        MSG_REALTIME_CONTROL => Jt1078Signal::RealtimeControl,
        MSG_PLAYBACK_REQUEST => Jt1078Signal::PlaybackRequest,
        MSG_PLAYBACK_CONTROL => Jt1078Signal::PlaybackControl,
        MSG_RESOURCE_LIST_REQUEST => Jt1078Signal::ResourceListRequest,
        MSG_RESOURCE_LIST_UPLOAD => Jt1078Signal::ResourceListUpload,
        MSG_FILE_UPLOAD_COMPLETE => Jt1078Signal::FileUploadComplete,
        other => Jt1078Signal::Unknown(other),
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Jt1078Error {
    #[error("invalid JT1078 magic")]
    Magic,
    #[error("packet is too short")]
    TooShort,
    #[error("body length mismatch")]
    Length,
    #[error("body exceeds u16")]
    BodyTooLarge,
    #[error("invalid signaling body")]
    SignalBody,
    #[error("invalid control body")]
    ControlBody,
    #[error("invalid playback body")]
    PlaybackBody,
    #[error("invalid playback control body")]
    PlaybackControlBody,
    #[error("invalid H.264 Annex-B data")]
    H264,
    #[error("invalid AAC ADTS frame")]
    Aac,
    #[error("invalid resource list request body")]
    ResourceBody,
    #[error("invalid resource list upload body")]
    ResourceUploadBody,
}

/// 30-byte JT/T 1078 media header + body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078Packet {
    pub flags: u8,
    pub marker_payload: u8,
    pub sequence: u16,
    pub sim: [u8; 6],
    pub channel: u8,
    pub data_type: u8,
    pub timestamp: u64,
    pub last_i_frame_interval: u16,
    pub last_frame_interval: u16,
    pub body: Vec<u8>,
}

/// JT/T 1078 payload type carried in the media header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jt1078Payload {
    VideoI,
    VideoP,
    VideoB,
    Audio,
    Transcoded,
    Unknown(u8),
}

impl Jt1078Packet {
    pub fn payload(&self) -> Jt1078Payload {
        match self.data_type {
            0 => Jt1078Payload::VideoI,
            1 => Jt1078Payload::VideoP,
            2 => Jt1078Payload::VideoB,
            3 => Jt1078Payload::Audio,
            4 => Jt1078Payload::Transcoded,
            other => Jt1078Payload::Unknown(other),
        }
    }

    pub fn is_keyframe(&self) -> bool {
        self.payload() == Jt1078Payload::VideoI
    }
}

impl Jt1078Packet {
    pub fn encode(&self) -> Result<Vec<u8>, Jt1078Error> {
        if self.body.len() > u16::MAX as usize {
            return Err(Jt1078Error::BodyTooLarge);
        }
        let mut out = Vec::with_capacity(30 + self.body.len());
        out.extend_from_slice(&JT1078_MAGIC);
        out.push(self.flags);
        out.push(self.marker_payload);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.sim);
        out.push(self.channel);
        out.push(self.data_type);
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.last_i_frame_interval.to_be_bytes());
        out.extend_from_slice(&self.last_frame_interval.to_be_bytes());
        out.extend_from_slice(&(self.body.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.body);
        Ok(out)
    }

    pub fn decode(raw: &[u8]) -> Result<Self, Jt1078Error> {
        if raw.len() < 30 {
            return Err(Jt1078Error::TooShort);
        }
        if raw[0..4] != JT1078_MAGIC {
            return Err(Jt1078Error::Magic);
        }
        let len = u16::from_be_bytes([raw[28], raw[29]]) as usize;
        if raw.len() != 30 + len {
            return Err(Jt1078Error::Length);
        }
        Ok(Self {
            flags: raw[4],
            marker_payload: raw[5],
            sequence: u16::from_be_bytes([raw[6], raw[7]]),
            sim: raw[8..14].try_into().map_err(|_| Jt1078Error::TooShort)?,
            channel: raw[14],
            data_type: raw[15],
            timestamp: u64::from_be_bytes([
                raw[16], raw[17], raw[18], raw[19], raw[20], raw[21], raw[22], raw[23],
            ]),
            last_i_frame_interval: u16::from_be_bytes([raw[24], raw[25]]),
            last_frame_interval: u16::from_be_bytes([raw[26], raw[27]]),
            body: raw[30..].to_vec(),
        })
    }
}

/// One 0x1205 uploaded resource item candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078ResourceItem {
    pub channel: u8,
    pub start_time: [u8; 6],
    pub end_time: [u8; 6],
    pub alarm_flags: u64,
    pub resource_type: u8,
    pub storage_type: u8,
    pub file_size: u32,
    pub path: String,
}

impl Jt1078ResourceItem {
    fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), Jt1078Error> {
        if self.path.len() > u8::MAX as usize {
            return Err(Jt1078Error::ResourceUploadBody);
        }
        out.push(self.channel);
        out.extend_from_slice(&self.start_time);
        out.extend_from_slice(&self.end_time);
        out.extend_from_slice(&self.alarm_flags.to_be_bytes());
        out.push(self.resource_type);
        out.push(self.storage_type);
        out.extend_from_slice(&self.file_size.to_be_bytes());
        out.push(self.path.len() as u8);
        out.extend_from_slice(self.path.as_bytes());
        Ok(())
    }

    fn decode_at(body: &[u8], offset: &mut usize) -> Result<Self, Jt1078Error> {
        if *offset + 28 > body.len() {
            return Err(Jt1078Error::ResourceUploadBody);
        }
        let start = *offset;
        let channel = body[start];
        let start_time = body[start + 1..start + 7]
            .try_into()
            .map_err(|_| Jt1078Error::ResourceUploadBody)?;
        let end_time = body[start + 7..start + 13]
            .try_into()
            .map_err(|_| Jt1078Error::ResourceUploadBody)?;
        let alarm_flags = u64::from_be_bytes(
            body[start + 13..start + 21]
                .try_into()
                .map_err(|_| Jt1078Error::ResourceUploadBody)?,
        );
        let resource_type = body[start + 21];
        let storage_type = body[start + 22];
        let file_size = u32::from_be_bytes(
            body[start + 23..start + 27]
                .try_into()
                .map_err(|_| Jt1078Error::ResourceUploadBody)?,
        );
        let path_len = body[start + 27] as usize;
        if start + 28 + path_len > body.len() {
            return Err(Jt1078Error::ResourceUploadBody);
        }
        let path = std::str::from_utf8(&body[start + 28..start + 28 + path_len])
            .map_err(|_| Jt1078Error::ResourceUploadBody)?
            .to_string();
        *offset = start + 28 + path_len;
        Ok(Self {
            channel,
            start_time,
            end_time,
            alarm_flags,
            resource_type,
            storage_type,
            file_size,
            path,
        })
    }
}

/// 0x9206 file upload command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078FileUploadCommand {
    pub server_ip: String,
    pub port: u16,
    pub user_name: String,
    pub password: String,
    pub upload_path: String,
    pub channel: u8,
    pub begin_time: [u8; 6],
    pub end_time: [u8; 6],
    pub alarm_flags: u64,
    pub media_type: u8,
    pub stream_type: u8,
    pub memory_position: u8,
    pub task_condition: u8,
}

impl Jt1078FileUploadCommand {
    pub fn encode(&self) -> Result<Vec<u8>, Jt1078Error> {
        let mut out = Vec::new();
        push_len_string(&mut out, &self.server_ip)?;
        out.extend_from_slice(&self.port.to_be_bytes());
        push_len_string(&mut out, &self.user_name)?;
        push_len_string(&mut out, &self.password)?;
        push_len_string(&mut out, &self.upload_path)?;
        out.push(self.channel);
        out.extend_from_slice(&self.begin_time);
        out.extend_from_slice(&self.end_time);
        out.extend_from_slice(&self.alarm_flags.to_be_bytes());
        out.push(self.media_type);
        out.push(self.stream_type);
        out.push(self.memory_position);
        out.push(self.task_condition);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        let mut offset = 0usize;
        let server_ip = read_len_string(body, &mut offset)?;
        let port = read_u16(body, &mut offset)?;
        let user_name = read_len_string(body, &mut offset)?;
        let password = read_len_string(body, &mut offset)?;
        let upload_path = read_len_string(body, &mut offset)?;
        if offset + 1 + 6 + 6 + 8 + 4 > body.len() {
            return Err(Jt1078Error::ControlBody);
        }
        let channel = body[offset];
        offset += 1;
        let begin_time = body[offset..offset + 6].try_into().unwrap();
        offset += 6;
        let end_time = body[offset..offset + 6].try_into().unwrap();
        offset += 6;
        let alarm_flags = u64::from_be_bytes(body[offset..offset + 8].try_into().unwrap());
        offset += 8;
        let media_type = body[offset];
        let stream_type = body[offset + 1];
        let memory_position = body[offset + 2];
        let task_condition = body[offset + 3];
        offset += 4;
        if offset != body.len() {
            return Err(Jt1078Error::ControlBody);
        }
        Ok(Self {
            server_ip,
            port,
            user_name,
            password,
            upload_path,
            channel,
            begin_time,
            end_time,
            alarm_flags,
            media_type,
            stream_type,
            memory_position,
            task_condition,
        })
    }
}

fn push_len_string(out: &mut Vec<u8>, value: &str) -> Result<(), Jt1078Error> {
    if value.len() > u8::MAX as usize {
        return Err(Jt1078Error::ControlBody);
    }
    out.push(value.len() as u8);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn read_len_string(body: &[u8], offset: &mut usize) -> Result<String, Jt1078Error> {
    if *offset >= body.len() {
        return Err(Jt1078Error::ControlBody);
    }
    let len = body[*offset] as usize;
    *offset += 1;
    if *offset + len > body.len() {
        return Err(Jt1078Error::ControlBody);
    }
    let value = String::from_utf8_lossy(&body[*offset..*offset + len]).into_owned();
    *offset += len;
    Ok(value)
}

fn read_u16(body: &[u8], offset: &mut usize) -> Result<u16, Jt1078Error> {
    if *offset + 2 > body.len() {
        return Err(Jt1078Error::ControlBody);
    }
    let value = u16::from_be_bytes([body[*offset], body[*offset + 1]]);
    *offset += 2;
    Ok(value)
}

/// 0x9207 file upload control: serial + 0 pause / 1 resume / 2 cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt1078FileUploadControl {
    pub msg_num: u16,
    pub control: u8,
}

impl Jt1078FileUploadControl {
    pub fn encode(&self) -> Result<Vec<u8>, Jt1078Error> {
        if self.control > 2 {
            return Err(Jt1078Error::ControlBody);
        }
        let mut out = Vec::with_capacity(3);
        out.extend_from_slice(&self.msg_num.to_be_bytes());
        out.push(self.control);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        if body.len() != 3 || body[2] > 2 {
            return Err(Jt1078Error::ControlBody);
        }
        Ok(Self {
            msg_num: u16::from_be_bytes([body[0], body[1]]),
            control: body[2],
        })
    }
}

/// 0x1206 file-upload-complete notification: serial + result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt1078FileUploadComplete {
    pub msg_num: u16,
    pub result: u8,
}

impl Jt1078FileUploadComplete {
    pub fn encode(&self) -> Result<Vec<u8>, Jt1078Error> {
        if self.result > 1 {
            return Err(Jt1078Error::ResourceBody);
        }
        let mut out = Vec::with_capacity(3);
        out.extend_from_slice(&self.msg_num.to_be_bytes());
        out.push(self.result);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        if body.len() != 3 || body[2] > 1 {
            return Err(Jt1078Error::ResourceBody);
        }
        Ok(Self {
            msg_num: u16::from_be_bytes([body[0], body[1]]),
            result: body[2],
        })
    }
}

/// 0x1205 uploaded resource list candidate: count u8 + items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078ResourceList {
    pub items: Vec<Jt1078ResourceItem>,
}

impl Jt1078ResourceList {
    pub fn encode(&self) -> Result<Vec<u8>, Jt1078Error> {
        if self.items.len() > u8::MAX as usize {
            return Err(Jt1078Error::ResourceUploadBody);
        }
        let mut out = Vec::new();
        out.push(self.items.len() as u8);
        for item in &self.items {
            item.encode_into(&mut out)?;
        }
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        let Some((&count, rest)) = body.split_first() else {
            return Err(Jt1078Error::ResourceUploadBody);
        };
        let mut items = Vec::with_capacity(count as usize);
        let mut offset = 0;
        for _ in 0..count {
            items.push(Jt1078ResourceItem::decode_at(rest, &mut offset)?);
        }
        if offset != rest.len() {
            return Err(Jt1078Error::ResourceUploadBody);
        }
        Ok(Self { items })
    }
}

/// 0x9205 resource list request candidate: channel + start/end BCD[6] + flags u64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt1078ResourceRequest {
    pub channel: u8,
    pub start_time: [u8; 6],
    pub end_time: [u8; 6],
    pub alarm_flags: u64,
}

impl Jt1078ResourceRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(21);
        out.push(self.channel);
        out.extend_from_slice(&self.start_time);
        out.extend_from_slice(&self.end_time);
        out.extend_from_slice(&self.alarm_flags.to_be_bytes());
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        if body.len() != 21 {
            return Err(Jt1078Error::ResourceBody);
        }
        Ok(Self {
            channel: body[0],
            start_time: body[1..7]
                .try_into()
                .map_err(|_| Jt1078Error::ResourceBody)?,
            end_time: body[7..13]
                .try_into()
                .map_err(|_| Jt1078Error::ResourceBody)?,
            alarm_flags: u64::from_be_bytes([
                body[13], body[14], body[15], body[16], body[17], body[18], body[19], body[20],
            ]),
        })
    }
}

/// Split an Annex-B buffer into NAL units (start codes removed).
pub fn split_annex_b(data: &[u8]) -> Result<Vec<&[u8]>, Jt1078Error> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        if data[index..].starts_with(&[0, 0, 1]) {
            starts.push((index, 3));
            index += 3;
        } else if index + 4 <= data.len() && data[index..].starts_with(&[0, 0, 0, 1]) {
            starts.push((index, 4));
            index += 4;
        } else {
            index += 1;
        }
    }
    if starts.is_empty() {
        return Err(Jt1078Error::H264);
    }
    let mut nals = Vec::with_capacity(starts.len());
    for (position, (offset, code_len)) in starts.iter().enumerate() {
        let begin = offset + code_len;
        let end = starts
            .get(position + 1)
            .map(|(next, _)| *next)
            .unwrap_or(data.len());
        if end <= begin {
            return Err(Jt1078Error::H264);
        }
        nals.push(&data[begin..end]);
    }
    Ok(nals)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078H264Frame {
    pub nals: Vec<Vec<u8>>,
}

impl Jt1078H264Frame {
    pub fn parse(data: &[u8]) -> Result<Self, Jt1078Error> {
        Ok(Self {
            nals: split_annex_b(data)?
                .into_iter()
                .map(<[u8]>::to_vec)
                .collect(),
        })
    }

    pub fn is_keyframe(&self) -> bool {
        self.nals
            .iter()
            .any(|nal| !nal.is_empty() && nal[0] & 0x1f == 5)
    }

    pub fn has_sps_pps(&self) -> bool {
        let has_sps = self
            .nals
            .iter()
            .any(|nal| !nal.is_empty() && nal[0] & 0x1f == 7);
        let has_pps = self
            .nals
            .iter()
            .any(|nal| !nal.is_empty() && nal[0] & 0x1f == 8);
        has_sps && has_pps
    }

    /// Rebuild an Annex-B buffer with 4-byte start codes, suitable for a
    /// downstream decoder or for persisting a keyframe.
    pub fn to_annex_b(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in &self.nals {
            if nal.is_empty() {
                continue;
            }
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078AacFrame {
    pub profile: u8,
    pub sample_rate_index: u8,
    pub channel_config: u8,
    pub frame_length: usize,
}

impl Jt1078AacFrame {
    pub fn parse(data: &[u8]) -> Result<Self, Jt1078Error> {
        if data.len() < 7 || data[0] != 0xFF || data[1] & 0xF0 != 0xF0 {
            return Err(Jt1078Error::Aac);
        }
        let profile = (data[2] >> 6) & 0x03;
        let sample_rate_index = (data[2] >> 2) & 0x0F;
        let channel_config = ((data[2] & 0x01) << 2) | ((data[3] >> 6) & 0x03);
        let frame_length = (((data[3] & 0x03) as usize) << 11)
            | ((data[4] as usize) << 3)
            | ((data[5] as usize) >> 5);
        if frame_length < 7 || frame_length > data.len() {
            return Err(Jt1078Error::Aac);
        }
        Ok(Self {
            profile,
            sample_rate_index,
            channel_config,
            frame_length,
        })
    }
}

/// 0x9202 playback control candidate: channel + command + speed + BCD[6].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt1078PlaybackControl {
    pub channel: u8,
    pub command: u8,
    pub speed: u8,
    pub time: [u8; 6],
}

impl Jt1078PlaybackControl {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9);
        out.push(self.channel);
        out.push(self.command);
        out.push(self.speed);
        out.extend_from_slice(&self.time);
        out
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        if body.len() != 9 {
            return Err(Jt1078Error::PlaybackControlBody);
        }
        Ok(Self {
            channel: body[0],
            command: body[1],
            speed: body[2],
            time: body[3..9]
                .try_into()
                .map_err(|_| Jt1078Error::PlaybackControlBody)?,
        })
    }
}

/// 0x9201 playback request candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078PlaybackRequest {
    pub server_ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub channel: u8,
    pub data_type: u8,
    pub stream_type: u8,
    pub playback_type: u8,
    pub start_time: [u8; 6],
    pub end_time: [u8; 6],
}

impl Jt1078PlaybackRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt1078Error> {
        if self.server_ip.is_empty() || self.server_ip.len() > u8::MAX as usize {
            return Err(Jt1078Error::PlaybackBody);
        }
        let mut out = Vec::with_capacity(1 + self.server_ip.len() + 20);
        out.push(self.server_ip.len() as u8);
        out.extend_from_slice(self.server_ip.as_bytes());
        out.extend_from_slice(&self.tcp_port.to_be_bytes());
        out.extend_from_slice(&self.udp_port.to_be_bytes());
        out.push(self.channel);
        out.push(self.data_type);
        out.push(self.stream_type);
        out.push(self.playback_type);
        out.extend_from_slice(&self.start_time);
        out.extend_from_slice(&self.end_time);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        if body.is_empty() {
            return Err(Jt1078Error::PlaybackBody);
        }
        let len = body[0] as usize;
        if len == 0 || body.len() != 1 + len + 20 {
            return Err(Jt1078Error::PlaybackBody);
        }
        let server_ip = std::str::from_utf8(&body[1..1 + len])
            .map_err(|_| Jt1078Error::PlaybackBody)?
            .to_string();
        let offset = 1 + len;
        Ok(Self {
            server_ip,
            tcp_port: u16::from_be_bytes([body[offset], body[offset + 1]]),
            udp_port: u16::from_be_bytes([body[offset + 2], body[offset + 3]]),
            channel: body[offset + 4],
            data_type: body[offset + 5],
            stream_type: body[offset + 6],
            playback_type: body[offset + 7],
            start_time: body[offset + 8..offset + 14]
                .try_into()
                .map_err(|_| Jt1078Error::PlaybackBody)?,
            end_time: body[offset + 14..offset + 20]
                .try_into()
                .map_err(|_| Jt1078Error::PlaybackBody)?,
        })
    }
}

/// 0x9102 real-time control candidate: channel + command + close type + stream type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jt1078RealtimeControl {
    pub channel: u8,
    pub command: u8,
    pub close_type: u8,
    pub stream_type: u8,
}

impl Jt1078RealtimeControl {
    pub fn encode(&self) -> Vec<u8> {
        vec![
            self.channel,
            self.command,
            self.close_type,
            self.stream_type,
        ]
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        if body.len() != 4 {
            return Err(Jt1078Error::ControlBody);
        }
        Ok(Self {
            channel: body[0],
            command: body[1],
            close_type: body[2],
            stream_type: body[3],
        })
    }
}

/// 0x9101 real-time request candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jt1078RealtimeRequest {
    pub server_ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub channel: u8,
    pub data_type: u8,
    pub stream_type: u8,
}

impl Jt1078RealtimeRequest {
    pub fn encode(&self) -> Result<Vec<u8>, Jt1078Error> {
        if self.server_ip.is_empty() || self.server_ip.len() > u8::MAX as usize {
            return Err(Jt1078Error::SignalBody);
        }
        let mut out = Vec::with_capacity(1 + self.server_ip.len() + 7);
        out.push(self.server_ip.len() as u8);
        out.extend_from_slice(self.server_ip.as_bytes());
        out.extend_from_slice(&self.tcp_port.to_be_bytes());
        out.extend_from_slice(&self.udp_port.to_be_bytes());
        out.push(self.channel);
        out.push(self.data_type);
        out.push(self.stream_type);
        Ok(out)
    }

    pub fn decode(body: &[u8]) -> Result<Self, Jt1078Error> {
        if body.is_empty() {
            return Err(Jt1078Error::SignalBody);
        }
        let len = body[0] as usize;
        if len == 0 || body.len() != 1 + len + 7 {
            return Err(Jt1078Error::SignalBody);
        }
        let server_ip = std::str::from_utf8(&body[1..1 + len])
            .map_err(|_| Jt1078Error::SignalBody)?
            .to_string();
        let offset = 1 + len;
        Ok(Self {
            server_ip,
            tcp_port: u16::from_be_bytes([body[offset], body[offset + 1]]),
            udp_port: u16::from_be_bytes([body[offset + 2], body[offset + 3]]),
            channel: body[offset + 4],
            data_type: body[offset + 5],
            stream_type: body[offset + 6],
        })
    }
}

/// Accumulates packets until the RTP marker bit closes a frame.
#[derive(Debug, Default)]
pub struct Jt1078FrameAssembler {
    frames: HashMap<StreamKey, Vec<u8>>,
    max_frame_bytes: usize,
}

impl Jt1078FrameAssembler {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            frames: HashMap::new(),
            max_frame_bytes,
        }
    }

    /// Append a packet body; returns a frame when the marker bit is set.
    pub fn push(&mut self, packet: &Jt1078Packet) -> Option<Vec<u8>> {
        let key = (packet.sim, packet.channel, packet.data_type);
        let frame = self.frames.entry(key).or_default();
        if frame.len() + packet.body.len() > self.max_frame_bytes {
            self.frames.remove(&key);
            return None;
        }
        frame.extend_from_slice(&packet.body);
        if packet.marker_payload & 0x80 != 0 {
            return self.frames.remove(&key);
        }
        None
    }

    pub fn pending(&self) -> usize {
        self.frames.len()
    }
}

type StreamKey = ([u8; 6], u8, u8);
type StreamBuffer = HashMap<StreamKey, BTreeMap<u16, Vec<u8>>>;

/// Per-stream sequence reorder buffer. It does not decode media.
#[derive(Debug, Default)]
pub struct Jt1078ReorderBuffer {
    streams: StreamBuffer,
    max_packets: usize,
}

impl Jt1078ReorderBuffer {
    pub fn new(max_packets: usize) -> Self {
        Self {
            streams: HashMap::new(),
            max_packets,
        }
    }

    /// Insert a packet payload; returns false when the packet is a duplicate or
    /// the global buffer limit is reached.
    pub fn push(&mut self, packet: &Jt1078Packet) -> bool {
        let key = (packet.sim, packet.channel, packet.data_type);
        if self
            .streams
            .get(&key)
            .is_some_and(|stream| stream.contains_key(&packet.sequence))
        {
            return false;
        }
        let total: usize = self.streams.values().map(BTreeMap::len).sum();
        if total >= self.max_packets {
            return false;
        }
        self.streams
            .entry(key)
            .or_default()
            .insert(packet.sequence, packet.body.clone());
        true
    }

    /// Drain one stream in ascending sequence order.
    pub fn drain(&mut self, sim: [u8; 6], channel: u8, data_type: u8) -> Vec<Vec<u8>> {
        self.streams
            .remove(&(sim, channel, data_type))
            .map(|stream| stream.into_values().collect())
            .unwrap_or_default()
    }

    pub fn pending(&self) -> usize {
        self.streams.values().map(BTreeMap::len).sum()
    }

    pub fn clear(&mut self) {
        self.streams.clear();
    }
}

/// Sequence-ordered stream pipeline: reorders packets per stream, then feeds
/// the marker-bit assembler. Out-of-order packets are held until the gap fills;
/// a bounded gap forces progress to avoid stalling forever.
#[derive(Debug)]
pub struct Jt1078StreamPipeline {
    expected: HashMap<StreamKey, u16>,
    pending: HashMap<StreamKey, BTreeMap<u16, Jt1078Packet>>,
    assembler: Jt1078FrameAssembler,
    max_packets: usize,
    max_gap: usize,
    stats: Jt1078StreamStats,
}

/// Weak-network observability counters for the 1078 stream pipeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Jt1078StreamStats {
    pub received: u64,
    pub duplicates: u64,
    pub dropped_limit: u64,
    pub gaps_skipped: u64,
    pub frames: u64,
}

impl Jt1078StreamPipeline {
    pub fn new(max_frame_bytes: usize, max_packets: usize, max_gap: usize) -> Self {
        Self {
            expected: HashMap::new(),
            pending: HashMap::new(),
            assembler: Jt1078FrameAssembler::new(max_frame_bytes),
            max_packets,
            max_gap: max_gap.max(1),
            stats: Jt1078StreamStats::default(),
        }
    }

    pub fn push(&mut self, packet: Jt1078Packet) -> Vec<Vec<u8>> {
        let key = (packet.sim, packet.channel, packet.data_type);
        self.stats.received += 1;
        let total: usize = self.pending.values().map(BTreeMap::len).sum();
        if total >= self.max_packets {
            self.stats.dropped_limit += 1;
            return Vec::new();
        }
        let mut expected = *self.expected.entry(key).or_insert(packet.sequence);
        let stream = self.pending.entry(key).or_default();
        if stream.contains_key(&packet.sequence) {
            self.stats.duplicates += 1;
            return Vec::new();
        }
        stream.insert(packet.sequence, packet);
        let mut frames = Vec::new();
        let mut iterations = 0usize;
        loop {
            iterations += 1;
            if iterations > self.max_packets + self.max_gap + 4 {
                tracing::error!(?key, expected, iterations, "JT1078 pipeline loop guard");
                break;
            }
            let mut stream = self.pending.remove(&key).unwrap_or_default();
            if let Some(packet) = stream.remove(&expected) {
                if let Some(frame) = self.assembler.push(&packet) {
                    self.stats.frames += 1;
                    frames.push(frame);
                }
                expected = expected.wrapping_add(1);
                self.expected.insert(key, expected);
                if !stream.is_empty() {
                    self.pending.insert(key, stream);
                }
                continue;
            }
            if stream.len() > self.max_gap {
                if let Some((&next, _)) = stream.iter().next() {
                    self.stats.gaps_skipped += 1;
                    // Advance the local cursor too; only updating the map
                    // would re-enter this branch forever.
                    expected = next;
                    self.expected.insert(key, expected);
                    self.pending.insert(key, stream);
                    continue;
                }
            }
            if !stream.is_empty() {
                self.pending.insert(key, stream);
            }
            break;
        }
        frames
    }

    pub fn pending_packets(&self) -> usize {
        self.pending.values().map(BTreeMap::len).sum()
    }

    pub fn pending_frames(&self) -> usize {
        self.assembler.pending()
    }

    pub fn stats(&self) -> Jt1078StreamStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Jt1078Packet {
        Jt1078Packet {
            flags: 0x80,
            marker_payload: 0x62,
            sequence: 7,
            sim: [0x89, 0x86, 0x00, 0x12, 0x34, 0x56],
            channel: 1,
            data_type: 0x01,
            timestamp: 1_700_000_000_000,
            last_i_frame_interval: 100,
            last_frame_interval: 40,
            body: vec![0, 0, 0, 1, 0x67],
        }
    }

    #[test]
    fn resource_list_upload_roundtrip() {
        let item = Jt1078ResourceItem {
            channel: 1,
            start_time: [0x26, 0x11, 0x19, 0x00, 0x00, 0x00],
            end_time: [0x26, 0x11, 0x19, 0x23, 0x59, 0x59],
            alarm_flags: 0,
            resource_type: 2,
            storage_type: 1,
            file_size: 4096,
            path: "CH1/20261119.mp4".to_string(),
        };
        let list = Jt1078ResourceList {
            items: vec![item.clone(), item],
        };
        assert_eq!(
            Jt1078ResourceList::decode(&list.encode().unwrap()).unwrap(),
            list
        );
        assert_eq!(
            Jt1078ResourceList::decode(&[1, 0]),
            Err(Jt1078Error::ResourceUploadBody)
        );
    }

    #[test]
    fn resource_request_body_roundtrip() {
        let request = Jt1078ResourceRequest {
            channel: 1,
            start_time: [0x26, 0x11, 0x19, 0x00, 0x00, 0x00],
            end_time: [0x26, 0x11, 0x19, 0x23, 0x59, 0x59],
            alarm_flags: 0,
        };
        assert_eq!(
            Jt1078ResourceRequest::decode(&request.encode()).unwrap(),
            request
        );
        assert_eq!(
            Jt1078ResourceRequest::decode(&[0; 20]),
            Err(Jt1078Error::ResourceBody)
        );
    }

    #[test]
    fn h264_annex_b_and_aac_frame_parse() {
        let h264 = [
            0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68, 0xCE, 0, 0, 1, 0x65, 0x88,
        ];
        let frame = Jt1078H264Frame::parse(&h264).unwrap();
        assert_eq!(frame.nals.len(), 3);
        assert_eq!(frame.nals[0][0] & 0x1f, 7);
        assert_eq!(frame.nals[1][0] & 0x1f, 8);
        assert!(frame.is_keyframe());

        // ADTS: 0xFFF, AAC-LC profile=1, 44.1kHz index=4, stereo=2, len=7.
        let aac = [0xFF, 0xF1, 0x50, 0x80, 0x00, 0xE0, 0xFC];
        let frame = Jt1078AacFrame::parse(&aac).unwrap();
        assert_eq!(frame.profile, 1);
        assert_eq!(frame.sample_rate_index, 4);
        assert_eq!(frame.channel_config, 2);
        assert_eq!(frame.frame_length, 7);
        assert_eq!(Jt1078AacFrame::parse(&[0; 7]), Err(Jt1078Error::Aac));
    }

    #[test]
    fn playback_control_body_roundtrip() {
        let control = Jt1078PlaybackControl {
            channel: 1,
            command: 2,
            speed: 1,
            time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
        };
        assert_eq!(
            Jt1078PlaybackControl::decode(&control.encode()).unwrap(),
            control
        );
        assert_eq!(
            Jt1078PlaybackControl::decode(&[0; 8]),
            Err(Jt1078Error::PlaybackControlBody)
        );
    }

    #[test]
    fn playback_request_body_roundtrip() {
        let request = Jt1078PlaybackRequest {
            server_ip: "127.0.0.1".to_string(),
            tcp_port: 1078,
            udp_port: 1079,
            channel: 1,
            data_type: 0,
            stream_type: 0,
            playback_type: 0,
            start_time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
            end_time: [0x26, 0x11, 0x19, 0x10, 0x30, 0x00],
        };
        assert_eq!(
            Jt1078PlaybackRequest::decode(&request.encode().unwrap()).unwrap(),
            request
        );
        assert_eq!(
            Jt1078PlaybackRequest::decode(&[3, b'1', b'2', b'3']),
            Err(Jt1078Error::PlaybackBody)
        );
    }

    #[test]
    fn realtime_control_body_roundtrip() {
        let control = Jt1078RealtimeControl {
            channel: 1,
            command: 0,
            close_type: 0,
            stream_type: 1,
        };
        assert_eq!(
            Jt1078RealtimeControl::decode(&control.encode()).unwrap(),
            control
        );
        assert_eq!(
            Jt1078RealtimeControl::decode(&[0; 3]),
            Err(Jt1078Error::ControlBody)
        );
    }

    #[test]
    fn frame_assembler_waits_for_marker_and_isolates_channels() {
        let mut assembler = Jt1078FrameAssembler::new(64);
        let mut first = sample();
        first.marker_payload = 0x62;
        first.sequence = 1;
        first.body = vec![1, 2];
        assert_eq!(assembler.push(&first), None);

        let mut second = first.clone();
        second.sequence = 2;
        second.body = vec![3];
        second.marker_payload = 0xE2;
        assert_eq!(assembler.push(&second), Some(vec![1, 2, 3]));

        let mut other_channel = first.clone();
        other_channel.channel = 2;
        assert_eq!(assembler.push(&other_channel), None);
        assert_eq!(assembler.pending(), 1);
    }

    #[test]
    fn realtime_request_body_roundtrip() {
        let request = Jt1078RealtimeRequest {
            server_ip: "127.0.0.1".to_string(),
            tcp_port: 1078,
            udp_port: 1079,
            channel: 1,
            data_type: 0,
            stream_type: 0,
        };
        let raw = request.encode().unwrap();
        assert_eq!(Jt1078RealtimeRequest::decode(&raw).unwrap(), request);
        assert_eq!(
            Jt1078RealtimeRequest::decode(&[4, b'1', b'2']),
            Err(Jt1078Error::SignalBody)
        );
    }

    #[test]
    fn signal_catalog_covers_known_ids() {
        assert_eq!(classify_signal(0x9101), Jt1078Signal::RealtimeRequest);
        assert_eq!(classify_signal(0x9102), Jt1078Signal::RealtimeControl);
        assert_eq!(classify_signal(0x9201), Jt1078Signal::PlaybackRequest);
        assert_eq!(classify_signal(0x9202), Jt1078Signal::PlaybackControl);
        assert_eq!(classify_signal(0x9205), Jt1078Signal::ResourceListRequest);
        assert_eq!(classify_signal(0x1205), Jt1078Signal::ResourceListUpload);
        assert_eq!(classify_signal(0x1234), Jt1078Signal::Unknown(0x1234));
    }

    #[test]
    fn reorder_buffer_sorts_and_deduplicates() {
        let mut buffer = Jt1078ReorderBuffer::new(8);
        let mut packets = Vec::new();
        for sequence in [3u16, 1, 2] {
            let mut packet = sample();
            packet.sequence = sequence;
            packet.body = vec![sequence as u8];
            assert!(buffer.push(&packet));
            packets.push(packet);
        }
        assert!(!buffer.push(&packets[0]));
        assert_eq!(buffer.pending(), 3);
        let drained = buffer.drain(packets[0].sim, packets[0].channel, packets[0].data_type);
        assert_eq!(drained, vec![vec![1], vec![2], vec![3]]);
        assert_eq!(buffer.pending(), 0);
    }

    #[test]
    fn media_packet_roundtrip_and_reject_bad_magic() {
        let packet = sample();
        let raw = packet.encode().unwrap();
        assert_eq!(Jt1078Packet::decode(&raw).unwrap(), packet);

        let mut bad = raw.clone();
        bad[0] = 0;
        assert_eq!(Jt1078Packet::decode(&bad), Err(Jt1078Error::Magic));
        assert_eq!(Jt1078Packet::decode(&raw[..20]), Err(Jt1078Error::TooShort));
    }

    #[test]
    fn h264_frame_rebuilds_annex_b_and_detects_parameter_sets() {
        let raw = [
            0, 0, 0, 1, 0x67, 0x42, 0, 0x1f, 0, 0, 0, 1, 0x68, 0xce, 0x38, 0x80, 0, 0, 0, 1, 0x65,
            0x88, 0x84,
        ];
        let frame = Jt1078H264Frame::parse(&raw).expect("parse");
        assert!(frame.has_sps_pps());
        assert!(frame.is_keyframe());
        assert_eq!(frame.nals.len(), 3);
        assert_eq!(frame.to_annex_b(), raw);
    }

    #[test]
    fn pipeline_reorders_before_assembling_frames() {
        let mut pipeline = Jt1078StreamPipeline::new(1024, 32, 8);
        let packet = |sequence, body: &[u8], marker: bool| Jt1078Packet {
            flags: 0x80,
            marker_payload: if marker { 0x80 } else { 0x00 },
            sequence,
            sim: [1, 2, 3, 4, 5, 6],
            channel: 1,
            data_type: 0x01,
            timestamp: 1,
            last_i_frame_interval: 0,
            last_frame_interval: 0,
            body: body.to_vec(),
        };
        assert!(pipeline.push(packet(0, b"a", false)).is_empty());
        assert!(pipeline.push(packet(2, b"c", true)).is_empty());
        let frames = pipeline.push(packet(1, b"b", false));
        assert_eq!(frames, vec![b"abc".to_vec()]);
        assert_eq!(pipeline.pending_packets(), 0);
    }

    #[test]
    fn file_upload_complete_roundtrips() {
        let value = Jt1078FileUploadComplete {
            msg_num: 7,
            result: 0,
        };
        assert_eq!(
            Jt1078FileUploadComplete::decode(&value.encode().unwrap()).unwrap(),
            value
        );
        assert!(Jt1078FileUploadComplete {
            msg_num: 7,
            result: 2
        }
        .encode()
        .is_err());
        assert!(Jt1078FileUploadComplete::decode(&[0, 7]).is_err());
    }

    #[test]
    fn file_upload_command_and_control_roundtrip() {
        let command = Jt1078FileUploadCommand {
            server_ip: "10.0.0.1".into(),
            port: 8080,
            user_name: "u".into(),
            password: "p".into(),
            upload_path: "/media/1".into(),
            channel: 1,
            begin_time: [0x26, 11, 23, 12, 0, 0],
            end_time: [0x26, 11, 23, 12, 30, 0],
            alarm_flags: 0,
            media_type: 0,
            stream_type: 0,
            memory_position: 0,
            task_condition: 1,
        };
        assert_eq!(
            Jt1078FileUploadCommand::decode(&command.encode().unwrap()).unwrap(),
            command
        );
        let control = Jt1078FileUploadControl {
            msg_num: 9,
            control: 2,
        };
        assert_eq!(
            Jt1078FileUploadControl::decode(&control.encode().unwrap()).unwrap(),
            control
        );
        assert!(Jt1078FileUploadControl {
            msg_num: 9,
            control: 3
        }
        .encode()
        .is_err());
    }

    #[test]
    fn media_payload_types_and_keyframes() {
        let mut packet = sample();
        packet.data_type = 0;
        assert_eq!(packet.payload(), Jt1078Payload::VideoI);
        assert!(packet.is_keyframe());
        packet.data_type = 3;
        assert_eq!(packet.payload(), Jt1078Payload::Audio);
        assert!(!packet.is_keyframe());
        packet.data_type = 9;
        assert_eq!(packet.payload(), Jt1078Payload::Unknown(9));
    }

    #[test]
    fn pipeline_reports_duplicates_and_emitted_frames() {
        let mut pipeline = Jt1078StreamPipeline::new(4096, 16, 4);
        let mut first = sample();
        first.sequence = 1;
        first.marker_payload = 0x80;
        first.body = b"keyframe".to_vec();
        let _ = pipeline.push(first.clone());
        let stats = pipeline.stats();
        assert_eq!(stats.received, 1);
        assert_eq!(stats.frames, 1);
        let mut limited = Jt1078StreamPipeline::new(4096, 0, 4);
        let _ = limited.push(first);
        assert_eq!(limited.stats().dropped_limit, 1);
    }
}
