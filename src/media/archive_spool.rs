//! Durable spool for archive segments whose upload retries were exhausted.
//!
//! Each segment is stored as a `.json` metadata file plus a `.bin` payload in
//! the spool directory. A background replay task retries these uploads; the
//! metadata file is written last and acts as the commit marker.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize, Deserialize)]
struct SpoolMeta {
    key: String,
    content_type: String,
}

#[derive(Debug)]
pub struct SpoolItem {
    pub id: String,
    pub key: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

pub static SPOOL_PENDING: AtomicU64 = AtomicU64::new(0);
pub static SPOOL_BYTES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct ArchiveSpool {
    dir: PathBuf,
    max_bytes: u64,
}

impl ArchiveSpool {
    pub fn new(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::new_with_limit(dir, spool_max_bytes())
    }

    pub fn new_with_limit(dir: impl AsRef<Path>, max_bytes: u64) -> std::io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            max_bytes: max_bytes.max(1),
        })
    }

    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("TSP_ARCHIVE_SPOOL_DIR").ok()?;
        let raw = raw.trim();
        if raw.is_empty() || matches!(raw.to_ascii_lowercase().as_str(), "off" | "false" | "0") {
            return None;
        }
        match Self::new(raw) {
            Ok(spool) => Some(spool),
            Err(error) => {
                tracing::warn!(%error, dir = raw, "archive spool directory unavailable");
                None
            }
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub async fn save(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> std::io::Result<()> {
        let length = bytes.len() as u64;
        let previous = SPOOL_BYTES.fetch_add(length, Ordering::Relaxed);
        if previous + length > self.max_bytes {
            SPOOL_BYTES.fetch_sub(length, Ordering::Relaxed);
            return Err(std::io::Error::other("archive spool is full"));
        }
        let id = spool_id(key);
        let meta = SpoolMeta {
            key: key.to_string(),
            content_type: content_type.to_string(),
        };
        let bin_path = self.dir.join(format!("{id}.bin"));
        let meta_path = self.dir.join(format!("{id}.json"));
        let bin_tmp = self.dir.join(format!("{id}.bin.tmp"));
        let meta_tmp = self.dir.join(format!("{id}.json.tmp"));
        let result = async {
            tokio::fs::write(&bin_tmp, bytes).await?;
            tokio::fs::rename(&bin_tmp, &bin_path).await?;
            let encoded = serde_json::to_vec(&meta).map_err(std::io::Error::other)?;
            tokio::fs::write(&meta_tmp, encoded).await?;
            tokio::fs::rename(&meta_tmp, &meta_path).await?;
            Ok::<(), std::io::Error>(())
        }
        .await;
        if let Err(error) = result {
            SPOOL_BYTES.fetch_sub(length, Ordering::Relaxed);
            return Err(error);
        }
        SPOOL_PENDING.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub async fn pending(&self) -> std::io::Result<Vec<SpoolItem>> {
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
            let Ok(meta) = serde_json::from_slice::<SpoolMeta>(&encoded) else {
                continue;
            };
            let bin_path = self.dir.join(format!("{id}.bin"));
            let Ok(bytes) = tokio::fs::read(&bin_path).await else {
                continue;
            };
            items.push(SpoolItem {
                id: id.to_string(),
                key: meta.key,
                content_type: meta.content_type,
                bytes,
            });
        }
        items.sort_by(|left, right| left.id.cmp(&right.id));
        SPOOL_PENDING.store(items.len() as u64, Ordering::Relaxed);
        SPOOL_BYTES.store(
            items.iter().map(|item| item.bytes.len() as u64).sum(),
            Ordering::Relaxed,
        );
        Ok(items)
    }

    pub async fn remove(&self, id: &str) -> std::io::Result<()> {
        let bin = self.dir.join(format!("{id}.bin"));
        let length = tokio::fs::metadata(&bin)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        let removed = tokio::fs::remove_file(self.dir.join(format!("{id}.json")))
            .await
            .is_ok();
        let _ = tokio::fs::remove_file(&bin).await;
        if removed {
            SPOOL_PENDING.fetch_sub(1, Ordering::Relaxed);
            SPOOL_BYTES.fetch_sub(length, Ordering::Relaxed);
        }
        Ok(())
    }
}

pub fn global() -> Option<&'static ArchiveSpool> {
    static SPOOL: OnceLock<Option<ArchiveSpool>> = OnceLock::new();
    SPOOL.get_or_init(ArchiveSpool::from_env).as_ref()
}

pub fn spool_max_bytes() -> u64 {
    std::env::var("TSP_ARCHIVE_SPOOL_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 1024 * 1024)
        .unwrap_or(1024 * 1024 * 1024)
}

pub fn replay_interval_seconds() -> u64 {
    std::env::var("TSP_ARCHIVE_SPOOL_REPLAY_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &u64| *value >= 10)
        .unwrap_or(60)
}

fn spool_id(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("tsp-spool-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn spool_save_pending_and_remove_roundtrip() {
        let dir = temp_dir();
        let spool = ArchiveSpool::new(&dir).expect("spool");
        spool
            .save("media/1/1/a.h264", b"video".to_vec(), "video/h264")
            .await
            .expect("save");
        spool
            .save("media/1/1/a.aac", b"audio".to_vec(), "audio/aac")
            .await
            .expect("save audio");
        let items = spool.pending().await.expect("pending");
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|item| item.key.ends_with(".h264")));
        assert!(items.iter().any(|item| item.key.ends_with(".aac")));
        for item in items {
            spool.remove(&item.id).await.expect("remove");
        }
        assert!(spool.pending().await.expect("empty").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spool_id_is_stable_and_path_safe() {
        let id = spool_id("media/../../evil.h264");
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(id, spool_id("media/../../evil.h264"));
    }

    #[tokio::test]
    async fn spool_rejects_when_limit_exceeded() {
        let dir = temp_dir();
        let spool = ArchiveSpool::new_with_limit(&dir, 10).expect("spool");
        spool
            .save("a", vec![0u8; 6], "video/h264")
            .await
            .expect("first");
        assert!(spool.save("b", vec![0u8; 6], "video/h264").await.is_err());
        assert_eq!(spool.pending().await.expect("pending").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
