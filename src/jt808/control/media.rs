//! JT/T 808 media/photo/audio control bodies (0x8800-0x8805, 0x0802/0x0805).

use super::super::codec::ProtocolError;

/// 0x0802 media search ACK item. The location is the fixed 28-byte basic
/// 0x0200 body; optional location TLVs are not included in this subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSearchItem {
    pub media_id: u32,
    pub media_type: u8,
    pub channel: u8,
    pub event: u8,
    pub location: [u8; 28],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSearchAck {
    pub serial: u16,
    pub total: u16,
    pub items: Vec<MediaSearchItem>,
}

pub fn encode_media_search_ack(ack: &MediaSearchAck) -> Result<Vec<u8>, ProtocolError> {
    if ack.items.len() > u16::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(6 + ack.items.len() * 35);
    body.extend_from_slice(&ack.serial.to_be_bytes());
    body.extend_from_slice(&ack.total.to_be_bytes());
    body.extend_from_slice(&(ack.items.len() as u16).to_be_bytes());
    for item in &ack.items {
        body.extend_from_slice(&item.media_id.to_be_bytes());
        body.push(item.media_type);
        body.push(item.channel);
        body.push(item.event);
        body.extend_from_slice(&item.location);
    }
    Ok(body)
}

pub fn decode_media_search_ack(body: &[u8]) -> Result<MediaSearchAck, ProtocolError> {
    if body.len() < 6 {
        return Err(ProtocolError::InvalidParameters);
    }
    let serial = u16::from_be_bytes([body[0], body[1]]);
    let total = u16::from_be_bytes([body[2], body[3]]);
    let count = u16::from_be_bytes([body[4], body[5]]) as usize;
    if body.len() != 6 + count * 35 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut items = Vec::with_capacity(count);
    let mut offset = 6;
    for _ in 0..count {
        items.push(MediaSearchItem {
            media_id: u32::from_be_bytes(body[offset..offset + 4].try_into().unwrap()),
            media_type: body[offset + 4],
            channel: body[offset + 5],
            event: body[offset + 6],
            location: body[offset + 7..offset + 35].try_into().unwrap(),
        });
        offset += 35;
    }
    Ok(MediaSearchAck {
        serial,
        total,
        items,
    })
}

/// 0x8802 media search request (JT/T 808-2019 table 76).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSearch {
    pub media_type: u8,
    pub channel: u8,
    pub event: u8,
    pub start: [u8; 6],
    pub end: [u8; 6],
}

pub fn encode_media_search(search: &MediaSearch) -> Vec<u8> {
    let mut body = Vec::with_capacity(15);
    body.push(search.media_type);
    body.push(search.channel);
    body.push(search.event);
    body.extend_from_slice(&search.start);
    body.extend_from_slice(&search.end);
    body
}

pub fn decode_media_search(body: &[u8]) -> Result<MediaSearch, ProtocolError> {
    if body.len() != 15 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(MediaSearch {
        media_type: body[0],
        channel: body[1],
        event: body[2],
        start: body[3..9].try_into().unwrap(),
        end: body[9..15].try_into().unwrap(),
    })
}

/// 0x8803 stored media upload command (JT/T 808-2019 table 79).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaUploadCommand {
    pub media_type: u8,
    pub channel: u8,
    pub event: u8,
    pub start: [u8; 6],
    pub end: [u8; 6],
    pub delete: u8,
}

pub fn encode_media_upload_command(command: &MediaUploadCommand) -> Vec<u8> {
    let mut body = Vec::with_capacity(16);
    body.push(command.media_type);
    body.push(command.channel);
    body.push(command.event);
    body.extend_from_slice(&command.start);
    body.extend_from_slice(&command.end);
    body.push(command.delete);
    body
}

pub fn decode_media_upload_command(body: &[u8]) -> Result<MediaUploadCommand, ProtocolError> {
    if body.len() != 16 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(MediaUploadCommand {
        media_type: body[0],
        channel: body[1],
        event: body[2],
        start: body[3..9].try_into().unwrap(),
        end: body[9..15].try_into().unwrap(),
        delete: body[15],
    })
}

/// 0x8804 audio recording command (JT/T 808-2019 table 80).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRecord {
    pub command: u8,
    pub seconds: u16,
    pub save: u8,
    pub sample_rate: u8,
}

pub fn encode_audio_record(record: &AudioRecord) -> Vec<u8> {
    vec![
        record.command,
        (record.seconds >> 8) as u8,
        record.seconds as u8,
        record.save,
        record.sample_rate,
    ]
}

pub fn decode_audio_record(body: &[u8]) -> Result<AudioRecord, ProtocolError> {
    if body.len() != 5 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(AudioRecord {
        command: body[0],
        seconds: u16::from_be_bytes([body[1], body[2]]),
        save: body[3],
        sample_rate: body[4],
    })
}

/// 0x8805 single stored media search/upload command (table 81).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleMediaUpload {
    pub media_id: u32,
    pub delete: u8,
}

pub fn encode_single_media_upload(command: &SingleMediaUpload) -> Vec<u8> {
    let mut body = Vec::with_capacity(5);
    body.extend_from_slice(&command.media_id.to_be_bytes());
    body.push(command.delete);
    body
}

pub fn decode_single_media_upload(body: &[u8]) -> Result<SingleMediaUpload, ProtocolError> {
    if body.len() != 5 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(SingleMediaUpload {
        media_id: u32::from_be_bytes(body[0..4].try_into().unwrap()),
        delete: body[4],
    })
}

/// 0x8900/0x0900 transparent message body: type(u8) + content.
pub fn encode_transparent(msg_type: u8, content: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + content.len());
    body.push(msg_type);
    body.extend_from_slice(content);
    body
}

pub fn decode_transparent(body: &[u8]) -> Result<(u8, &[u8]), ProtocolError> {
    let Some((&msg_type, content)) = body.split_first() else {
        return Err(ProtocolError::InvalidParameters);
    };
    Ok((msg_type, content))
}

/// 0x8800 media upload ACK (JT/T 808-2019 table 73).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaUploadAck {
    pub media_id: u32,
    pub retransmit: Vec<u8>,
}

pub fn encode_media_upload_ack(ack: &MediaUploadAck) -> Result<Vec<u8>, ProtocolError> {
    if ack.retransmit.len() > 125 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(5 + ack.retransmit.len());
    body.extend_from_slice(&ack.media_id.to_be_bytes());
    body.push(ack.retransmit.len() as u8);
    body.extend_from_slice(&ack.retransmit);
    Ok(body)
}

pub fn decode_media_upload_ack(body: &[u8]) -> Result<MediaUploadAck, ProtocolError> {
    if body.len() < 5 {
        return Err(ProtocolError::InvalidParameters);
    }
    let media_id = u32::from_be_bytes(body[0..4].try_into().unwrap());
    let count = body[4] as usize;
    if count > 125 || body.len() != 5 + count {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(MediaUploadAck {
        media_id,
        retransmit: body[5..].to_vec(),
    })
}

/// 0x8801 camera immediate capture (JT/T 808-2019 table 74).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraCapture {
    pub channel: u8,
    pub command: u16,
    pub interval: u16,
    pub save: u8,
    pub resolution: u8,
    pub quality: u8,
    pub contrast: u8,
    pub saturation: u8,
    pub chroma: u8,
}

pub fn encode_camera_capture(command: &CameraCapture) -> Vec<u8> {
    let mut body = Vec::with_capacity(11);
    body.push(command.channel);
    body.extend_from_slice(&command.command.to_be_bytes());
    body.extend_from_slice(&command.interval.to_be_bytes());
    body.extend_from_slice(&[
        command.save,
        command.resolution,
        command.quality,
        command.contrast,
        command.saturation,
        command.chroma,
    ]);
    body
}

pub fn decode_camera_capture(body: &[u8]) -> Result<CameraCapture, ProtocolError> {
    if body.len() != 11 {
        return Err(ProtocolError::InvalidParameters);
    }
    Ok(CameraCapture {
        channel: body[0],
        command: u16::from_be_bytes([body[1], body[2]]),
        interval: u16::from_be_bytes([body[3], body[4]]),
        save: body[5],
        resolution: body[6],
        quality: body[7],
        contrast: body[8],
        saturation: body[9],
        chroma: body[10],
    })
}

/// 0x0805 camera capture ACK (JT/T 808-2019 table 75).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraCaptureAck {
    pub serial: u16,
    pub result: u8,
    pub media_ids: Vec<u32>,
}

pub fn encode_camera_capture_ack(ack: &CameraCaptureAck) -> Result<Vec<u8>, ProtocolError> {
    if ack.media_ids.len() > u8::MAX as usize {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut body = Vec::with_capacity(4 + ack.media_ids.len() * 4);
    body.extend_from_slice(&ack.serial.to_be_bytes());
    body.push(ack.result);
    body.push(ack.media_ids.len() as u8);
    for id in &ack.media_ids {
        body.extend_from_slice(&id.to_be_bytes());
    }
    Ok(body)
}

pub fn decode_camera_capture_ack(body: &[u8]) -> Result<CameraCaptureAck, ProtocolError> {
    if body.len() < 4 {
        return Err(ProtocolError::InvalidParameters);
    }
    let serial = u16::from_be_bytes([body[0], body[1]]);
    let result = body[2];
    let count = body[3] as usize;
    if body.len() != 4 + count * 4 {
        return Err(ProtocolError::InvalidParameters);
    }
    let mut media_ids = Vec::with_capacity(count);
    for chunk in body[4..].chunks(4) {
        media_ids.push(u32::from_be_bytes(chunk.try_into().unwrap()));
    }
    Ok(CameraCaptureAck {
        serial,
        result,
        media_ids,
    })
}
