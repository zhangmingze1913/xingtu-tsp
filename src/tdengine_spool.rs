//! Durable spool for TDengine samples that exhausted their retry budget.
//!
//! One JSON file per sample keeps the format simple and avoids a shared index.
//! Files are named by UUID and written through a temporary file.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

use crate::tdengine::Sample;

pub static SPOOLED: AtomicU64 = AtomicU64::new(0);
pub static REPLAYED: AtomicU64 = AtomicU64::new(0);
pub static SPOOL_FAILED: AtomicU64 = AtomicU64::new(0);
pub static SPOOL_PENDING: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct TdengineSpool {
    dir: PathBuf,
    max_files: usize,
}

impl TdengineSpool {
    pub fn new(dir: impl AsRef<Path>, max_files: usize) -> std::io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            max_files: max_files.max(1),
        })
    }

    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("TSP_TDENGINE_SPOOL_DIR").ok()?;
        let raw = raw.trim();
        if raw.is_empty() || matches!(raw.to_ascii_lowercase().as_str(), "off" | "false" | "0") {
            return None;
        }
        let max_files = std::env::var("TSP_TDENGINE_SPOOL_MAX_FILES")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value: &usize| *value >= 1)
            .unwrap_or(100_000);
        match Self::new(raw, max_files) {
            Ok(spool) => Some(spool),
            Err(error) => {
                tracing::warn!(%error, dir = raw, "TDengine spool directory unavailable");
                None
            }
        }
    }

    pub async fn save(&self, sample: &Sample) -> std::io::Result<()> {
        let pending = self.count().await?;
        if pending >= self.max_files {
            return Err(std::io::Error::other("TDengine spool is full"));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let tmp = self.dir.join(format!("{id}.json.tmp"));
        let path = self.dir.join(format!("{id}.json"));
        let encoded = serde_json::to_vec(sample).map_err(std::io::Error::other)?;
        tokio::fs::write(&tmp, encoded).await?;
        tokio::fs::rename(&tmp, &path).await?;
        SPOOLED.fetch_add(1, Ordering::Relaxed);
        SPOOL_PENDING.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub async fn pending(&self) -> std::io::Result<Vec<(String, Sample)>> {
        let mut items = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let Ok(encoded) = tokio::fs::read(&path).await else {
                continue;
            };
            if let Ok(sample) = serde_json::from_slice::<Sample>(&encoded) {
                items.push((id.to_string(), sample));
            }
        }
        items.sort_by(|left, right| left.0.cmp(&right.0));
        SPOOL_PENDING.store(items.len() as u64, Ordering::Relaxed);
        Ok(items)
    }

    pub async fn remove(&self, id: &str) -> std::io::Result<()> {
        let removed = tokio::fs::remove_file(self.dir.join(format!("{id}.json")))
            .await
            .is_ok();
        if removed {
            SPOOL_PENDING.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }

    pub async fn count(&self) -> std::io::Result<usize> {
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        let mut count = 0;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                count += 1;
            }
        }
        Ok(count)
    }
}

pub fn global() -> Option<&'static TdengineSpool> {
    static SPOOL: OnceLock<Option<TdengineSpool>> = OnceLock::new();
    SPOOL.get_or_init(TdengineSpool::from_env).as_ref()
}

pub fn replay_interval_seconds() -> u64 {
    std::env::var("TSP_TDENGINE_SPOOL_REPLAY_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &u64| *value >= 10)
        .unwrap_or(60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sample {
        Sample {
            vehicle_id: "veh-spool".to_string(),
            ts_ms: 1_700_000_000_000,
            lat: 31.2,
            lon: 121.4,
            speed: 50.0,
            heading: 90.0,
            ignition: true,
            soc: 80.0,
            odometer_km: 100.0,
        }
    }

    #[tokio::test]
    async fn spool_save_pending_remove_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tsp-td-spool-{}", uuid::Uuid::new_v4()));
        let spool = TdengineSpool::new(&dir, 10).expect("spool");
        spool.save(&sample()).await.expect("save");
        let items = spool.pending().await.expect("pending");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1.vehicle_id, "veh-spool");
        spool.remove(&items[0].0).await.expect("remove");
        assert!(spool.pending().await.expect("empty").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn spool_enforces_max_files() {
        let dir = std::env::temp_dir().join(format!("tsp-td-limit-{}", uuid::Uuid::new_v4()));
        let spool = TdengineSpool::new(&dir, 1).expect("spool");
        spool.save(&sample()).await.expect("first");
        assert!(spool.save(&sample()).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
