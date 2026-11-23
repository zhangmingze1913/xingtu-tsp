//! In-process JT/T 1078 media session hub. A gateway/UDP listener can call
//! `ingest`; the HTTP layer reads the latest video/audio payload per SIM+channel.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Instant,
};

use crate::standards::jt1078::Jt1078Packet;

use super::playback::PlaybackSession;

#[derive(Default)]
struct Session {
    playback: PlaybackSession,
    video: Vec<u8>,
    audio: Vec<u8>,
    pending_video: Vec<u8>,
    pending_audio: Vec<u8>,
    header: Vec<u8>,
    updated: Option<Instant>,
}

#[derive(Default)]
pub struct MediaHub {
    sessions: Mutex<HashMap<(String, u8), Session>>,
}

const MAX_BUFFER: usize = 4 * 1024 * 1024;

impl MediaHub {
    pub fn ingest(&self, packet: &Jt1078Packet) -> bool {
        let key = (hex_sim(&packet.sim), packet.channel);
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        if sessions.len() > 256 {
            sessions.retain(|_, session| {
                session
                    .updated
                    .is_some_and(|at| at.elapsed().as_secs() < 60)
            });
        }
        let session = sessions.entry(key).or_default();
        let Ok(frame) = session.playback.push(packet) else {
            return false;
        };
        let Some(frame) = frame else {
            return false;
        };
        if matches!(packet.data_type, 0 | 2) {
            if contains_parameter_sets(&frame.annex_b) {
                session.header = frame.annex_b.clone();
            }
            if session.video.len() + frame.annex_b.len() > MAX_BUFFER {
                session.video.clear();
            }
            session.video.extend_from_slice(&frame.annex_b);
            session.pending_video.extend_from_slice(&frame.annex_b);
        } else {
            if session.audio.len() + frame.annex_b.len() > MAX_BUFFER {
                session.audio.clear();
            }
            session.audio.extend_from_slice(&frame.annex_b);
            session.pending_audio.extend_from_slice(&frame.annex_b);
        }
        session.updated = Some(Instant::now());
        true
    }

    pub fn snapshot(&self, sim: &str, channel: u8) -> Option<(Vec<u8>, Vec<u8>)> {
        let sessions = self.sessions.lock().ok()?;
        let session = sessions.get(&(sim.to_string(), channel))?;
        if session.video.is_empty() && session.audio.is_empty() {
            return None;
        }
        Some((session.video.clone(), session.audio.clone()))
    }

    /// Take newly arrived media and leave the session history untouched. Only
    /// one streaming consumer should call this per session.
    pub fn take(&self, sim: &str, channel: u8) -> Option<(Vec<u8>, Vec<u8>)> {
        let mut sessions = self.sessions.lock().ok()?;
        let session = sessions.get_mut(&(sim.to_string(), channel))?;
        if session.pending_video.is_empty() && session.pending_audio.is_empty() {
            return None;
        }
        let mut video = std::mem::take(&mut session.pending_video);
        let audio = std::mem::take(&mut session.pending_audio);
        if !video.is_empty() && !session.header.is_empty() {
            let mut with_header = session.header.clone();
            with_header.extend_from_slice(&video);
            video = with_header;
        }
        Some((video, audio))
    }

    pub fn clear(&self, sim: &str, channel: u8) -> bool {
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        sessions.remove(&(sim.to_string(), channel)).is_some()
    }

    pub fn list(&self) -> Vec<(String, u8, usize, usize)> {
        let Ok(sessions) = self.sessions.lock() else {
            return Vec::new();
        };
        let mut items: Vec<_> = sessions
            .iter()
            .map(|((sim, channel), session)| {
                (
                    sim.clone(),
                    *channel,
                    session.video.len(),
                    session.audio.len(),
                )
            })
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        items
    }

    pub fn sessions(&self) -> usize {
        self.sessions.lock().map(|s| s.len()).unwrap_or(0)
    }
}

pub fn global() -> &'static MediaHub {
    static HUB: OnceLock<MediaHub> = OnceLock::new();
    HUB.get_or_init(MediaHub::default)
}

fn contains_parameter_sets(data: &[u8]) -> bool {
    let mut index = 0;
    while index + 4 <= data.len() {
        if data[index..].starts_with(&[0, 0, 0, 1]) && index + 4 < data.len() {
            let nal = data[index + 4] & 0x1f;
            if nal == 7 || nal == 8 {
                return true;
            }
        }
        index += 1;
    }
    false
}

pub fn hex_sim(sim: &[u8; 6]) -> String {
    sim.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(marker: bool, body: &[u8]) -> Jt1078Packet {
        Jt1078Packet {
            flags: 0,
            marker_payload: if marker { 0x80 } else { 0 },
            sequence: 1,
            sim: [0x13, 0x80, 0x00, 0x00, 0x00, 0x01],
            channel: 1,
            data_type: 0,
            timestamp: 1000,
            last_i_frame_interval: 0,
            last_frame_interval: 0,
            body: body.to_vec(),
        }
    }

    #[test]
    fn hub_keeps_latest_media_per_sim_and_channel() {
        let hub = MediaHub::default();
        let idr = [0, 0, 0, 1, 0x65, 0x88, 0x84];
        assert!(hub.ingest(&packet(true, &idr)));
        let sim = hex_sim(&[0x13, 0x80, 0x00, 0x00, 0x00, 0x01]);
        assert_eq!(sim, "138000000001");
        let (video, audio) = hub.snapshot(&sim, 1).expect("snapshot");
        assert_eq!(video, idr);
        assert!(audio.is_empty());
        assert_eq!(hub.sessions(), 1);
        assert_eq!(hub.list(), vec![(sim.clone(), 1, idr.len(), 0)]);
        let (pending, _) = hub.take(&sim, 1).expect("pending");
        assert_eq!(pending, idr);
        assert!(hub.take(&sim, 1).is_none());
        assert!(hub.clear(&sim, 1));
        assert!(!hub.clear(&sim, 1));
        assert!(hub.snapshot("000000000000", 1).is_none());
    }
}
