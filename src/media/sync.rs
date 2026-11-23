//! Audio/video clock alignment for JT/T 1078 playback.

#[derive(Debug, Default)]
pub struct AvSync {
    video_ts: Option<u64>,
    audio_ts: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    KeepBoth,
    WaitVideo,
    WaitAudio,
}

impl AvSync {
    pub fn observe(&mut self, data_type: u8, timestamp: u64) {
        if matches!(data_type, 0 | 2) {
            self.video_ts = Some(timestamp);
        } else {
            self.audio_ts = Some(timestamp);
        }
    }

    /// Positive means video leads audio.
    pub fn drift_ms(&self) -> Option<i64> {
        match (self.video_ts, self.audio_ts) {
            (Some(video), Some(audio)) => Some(video as i64 - audio as i64),
            _ => None,
        }
    }

    pub fn action(&self, tolerance_ms: i64) -> SyncAction {
        match self.drift_ms() {
            Some(drift) if drift > tolerance_ms => SyncAction::WaitAudio,
            Some(drift) if drift < -tolerance_ms => SyncAction::WaitVideo,
            _ => SyncAction::KeepBoth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn av_sync_reports_drift_and_wait_side() {
        let mut sync = AvSync::default();
        sync.observe(0, 1_000);
        assert_eq!(sync.drift_ms(), None);
        sync.observe(1, 980);
        assert_eq!(sync.drift_ms(), Some(20));
        assert_eq!(sync.action(40), SyncAction::KeepBoth);
        assert_eq!(sync.action(10), SyncAction::WaitAudio);

        sync.observe(1, 1_100);
        assert_eq!(sync.action(40), SyncAction::WaitVideo);
    }
}
