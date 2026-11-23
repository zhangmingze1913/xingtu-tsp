//! JT/T 1078 playback pipeline: packet reassembly, frame classification and
//! dispatch to an injected decoder/renderer. The codec itself is a trait
//! boundary so a real H.264/AAC decoder can be added without changing the
//! transport path.

use super::sync::AvSync;
use crate::standards::jt1078::{
    Jt1078AacFrame, Jt1078Error, Jt1078FrameAssembler, Jt1078H264Frame, Jt1078Packet,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackFrame {
    pub sim: [u8; 6],
    pub channel: u8,
    pub data_type: u8,
    pub timestamp: u64,
    pub keyframe: bool,
    pub drift_ms: Option<i64>,
    pub annex_b: Vec<u8>,
}

pub trait PlaybackSink {
    fn on_frame(&mut self, frame: &PlaybackFrame);
}

#[derive(Debug)]
pub struct PlaybackSession {
    assembler: Jt1078FrameAssembler,
    frames: u64,
    keyframes: u64,
    av: AvSync,
}

impl Default for PlaybackSession {
    fn default() -> Self {
        Self::new(4 * 1024 * 1024)
    }
}

impl PlaybackSession {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            assembler: Jt1078FrameAssembler::new(max_frame_bytes),
            frames: 0,
            keyframes: 0,
            av: AvSync::default(),
        }
    }

    pub fn push(&mut self, packet: &Jt1078Packet) -> Result<Option<PlaybackFrame>, Jt1078Error> {
        let Some(body) = self.assembler.push(packet) else {
            return Ok(None);
        };
        let keyframe = if is_video(packet.data_type) {
            match Jt1078H264Frame::parse(&body) {
                Ok(frame) => frame.is_keyframe(),
                Err(_) => false,
            }
        } else {
            false
        };
        if !is_video(packet.data_type) && Jt1078AacFrame::parse(&body).is_err() {
            return Err(Jt1078Error::Aac);
        }
        self.av.observe(packet.data_type, packet.timestamp);
        self.frames += 1;
        if keyframe {
            self.keyframes += 1;
        }
        Ok(Some(PlaybackFrame {
            sim: packet.sim,
            channel: packet.channel,
            data_type: packet.data_type,
            timestamp: packet.timestamp,
            keyframe,
            drift_ms: self.av.drift_ms(),
            annex_b: body,
        }))
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn keyframes(&self) -> u64 {
        self.keyframes
    }

    pub fn drift_ms(&self) -> Option<i64> {
        self.av.drift_ms()
    }
}

fn is_video(data_type: u8) -> bool {
    matches!(data_type, 0 | 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(sequence: u16, marker: bool, body: &[u8]) -> Jt1078Packet {
        Jt1078Packet {
            flags: 0,
            marker_payload: if marker { 0x80 } else { 0 },
            sequence,
            sim: [1, 2, 3, 4, 5, 6],
            channel: 1,
            data_type: 0,
            timestamp: 1000 + sequence as u64,
            last_i_frame_interval: 0,
            last_frame_interval: 0,
            body: body.to_vec(),
        }
    }

    #[test]
    fn playback_reassembles_and_marks_keyframe() {
        let sps = [0, 0, 0, 1, 0x67, 0x42, 0, 0x1f];
        let pps = [0, 0, 0, 1, 0x68, 0xce, 0x38, 0x80];
        let idr = [0, 0, 0, 1, 0x65, 0x88, 0x84];
        let mut session = PlaybackSession::new(1024 * 1024);
        assert!(session.push(&packet(1, false, &sps)).unwrap().is_none());
        assert!(session.push(&packet(2, false, &pps)).unwrap().is_none());
        let frame = session
            .push(&packet(3, true, &idr))
            .unwrap()
            .expect("frame");
        assert_eq!(frame.sim, [1, 2, 3, 4, 5, 6]);
        assert!(frame.keyframe);
        assert_eq!(frame.drift_ms, None);
        assert_eq!(
            frame.annex_b,
            [sps.as_slice(), pps.as_slice(), idr.as_slice()].concat()
        );
        assert_eq!(session.frames(), 1);
        assert_eq!(session.keyframes(), 1);
    }
}
