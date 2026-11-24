//! RustFS/S3 archive for recorded JT/T 1078 media.
//!
//! Recording is intentionally raw: H.264 Annex-B video and AAC ADTS audio are
//! uploaded as separate objects. Playback remuxes them on demand, so ingest
//! does not pay an FFmpeg cost per segment.
//!
//! Configuration is environment based; when `TSP_RUSTFS_ARCHIVE` is not truthy
//! the feature is disabled and the media hub keeps its existing in-memory-only
//! behaviour.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::Duration,
};

use aws_sdk_s3::{
    config::{retry::RetryConfig, BehaviorVersion, Credentials, Region},
    primitives::ByteStream,
    Client,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::Semaphore;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("archive is not configured")]
    Disabled,
    #[error("invalid archive key: {0}")]
    InvalidKey(String),
    #[error("S3 request failed: {0}")]
    S3(String),
    #[error("S3 upload timed out after {0:?}")]
    Timeout(Duration),
}

pub static UPLOAD_SUCCESS: AtomicU64 = AtomicU64::new(0);
pub static UPLOAD_FAILURE: AtomicU64 = AtomicU64::new(0);
pub static UPLOAD_RETRIES: AtomicU64 = AtomicU64::new(0);
pub static SEGMENTS_DROPPED: AtomicU64 = AtomicU64::new(0);
pub static CLEANUP_DELETED: AtomicU64 = AtomicU64::new(0);
pub static PLAYBACK_SUCCESS: AtomicU64 = AtomicU64::new(0);
pub static PLAYBACK_FAILURE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default)]
pub struct ArchiveStats {
    pub upload_success: u64,
    pub upload_failure: u64,
    pub upload_retries: u64,
    pub segments_dropped: u64,
    pub cleanup_deleted: u64,
    pub playback_success: u64,
    pub playback_failure: u64,
}

pub fn stats() -> ArchiveStats {
    ArchiveStats {
        upload_success: UPLOAD_SUCCESS.load(Ordering::Relaxed),
        upload_failure: UPLOAD_FAILURE.load(Ordering::Relaxed),
        upload_retries: UPLOAD_RETRIES.load(Ordering::Relaxed),
        segments_dropped: SEGMENTS_DROPPED.load(Ordering::Relaxed),
        cleanup_deleted: CLEANUP_DELETED.load(Ordering::Relaxed),
        playback_success: PLAYBACK_SUCCESS.load(Ordering::Relaxed),
        playback_failure: PLAYBACK_FAILURE.load(Ordering::Relaxed),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchivedObject {
    pub key: String,
    pub size: i64,
    /// LastModified as Unix seconds when S3 reports it.
    pub modified_epoch: Option<i64>,
}

#[derive(Clone)]
pub struct RustFsArchive {
    client: Client,
    bucket: String,
    prefix: String,
    upload_slots: Arc<Semaphore>,
    retries: u32,
    timeout: Duration,
}

impl RustFsArchive {
    pub fn from_env() -> Option<Self> {
        if !enabled() {
            return None;
        }
        let endpoint = std::env::var("TSP_RUSTFS_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:9000".to_string());
        let bucket = std::env::var("TSP_RUSTFS_BUCKET").unwrap_or_else(|_| "tsp-media".to_string());
        let access =
            std::env::var("TSP_RUSTFS_ACCESS_KEY").or_else(|_| std::env::var("RUSTFS_ACCESS_KEY"));
        let secret =
            std::env::var("TSP_RUSTFS_SECRET_KEY").or_else(|_| std::env::var("RUSTFS_SECRET_KEY"));
        let (Ok(access), Ok(secret)) = (access, secret) else {
            tracing::warn!(
                "TSP_RUSTFS_ARCHIVE is enabled but TSP_RUSTFS_ACCESS_KEY/SECRET_KEY are missing"
            );
            return None;
        };
        let region = std::env::var("TSP_RUSTFS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let prefix = std::env::var("TSP_RUSTFS_PREFIX").unwrap_or_else(|_| "media".to_string());
        match Self::from_parts(&endpoint, &bucket, &access, &secret, &region, &prefix) {
            Ok(archive) => {
                tracing::info!(%endpoint, %bucket, %prefix, "RustFS media archive enabled");
                Some(archive)
            }
            Err(error) => {
                tracing::warn!(%error, "RustFS media archive disabled");
                None
            }
        }
    }

    pub fn from_parts(
        endpoint: &str,
        bucket: &str,
        access: &str,
        secret: &str,
        region: &str,
        prefix: &str,
    ) -> Result<Self, ArchiveError> {
        let endpoint = endpoint.trim().trim_end_matches('/');
        let bucket = bucket.trim();
        if endpoint.is_empty() || bucket.is_empty() || access.is_empty() || secret.is_empty() {
            return Err(ArchiveError::InvalidKey(
                "endpoint, bucket and credentials are required".to_string(),
            ));
        }
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region.to_string()))
            .endpoint_url(endpoint)
            .credentials_provider(Credentials::new(
                access.to_string(),
                secret.to_string(),
                None,
                None,
                "rustfs",
            ))
            .force_path_style(true)
            .retry_config(RetryConfig::disabled())
            .build();
        let retries = env_u32("TSP_RUSTFS_UPLOAD_RETRIES", 3, 0, 10);
        let timeout =
            Duration::from_secs(env_u32("TSP_RUSTFS_UPLOAD_TIMEOUT_SECS", 30, 1, 600) as u64);
        let concurrency = env_u32("TSP_RUSTFS_UPLOAD_CONCURRENCY", 4, 1, 64) as usize;
        Ok(Self {
            client: Client::from_conf(config),
            bucket: bucket.to_string(),
            prefix: normalize_prefix(prefix),
            upload_slots: Arc::new(Semaphore::new(concurrency)),
            retries,
            timeout,
        })
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Object key for one raw media segment.
    pub fn media_key(
        &self,
        sim: &str,
        channel: u8,
        at: DateTime<Utc>,
        sequence: u64,
        extension: &str,
    ) -> String {
        format!(
            "{}/{}/{}/{:04}/{:02}/{:02}/{:02}{:02}{:02}-{:06}.{}",
            self.prefix,
            safe_component(sim),
            channel,
            at.format("%Y"),
            at.format("%m"),
            at.format("%d"),
            at.format("%H"),
            at.format("%M"),
            at.format("%S"),
            sequence,
            extension
        )
    }

    pub fn media_prefix(&self, sim: &str, channel: u8) -> String {
        format!("{}/{}/{}/", self.prefix, safe_component(sim), channel)
    }

    /// Override retry count and per-attempt timeout (tests/operators).
    pub fn with_upload_policy(mut self, retries: u32, timeout: Duration) -> Self {
        self.retries = retries;
        self.timeout = timeout;
        self
    }

    /// Upload with bounded concurrency, timeout and exponential backoff.
    pub async fn put_with_retry(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), ArchiveError> {
        let _permit = self
            .upload_slots
            .acquire()
            .await
            .map_err(|error| ArchiveError::S3(error.to_string()))?;
        let mut last_error = None;
        for attempt in 0..=self.retries {
            match tokio::time::timeout(self.timeout, self.put(key, bytes.clone(), content_type))
                .await
            {
                Ok(Ok(())) => {
                    UPLOAD_SUCCESS.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
                Ok(Err(error)) => last_error = Some(error),
                Err(_) => last_error = Some(ArchiveError::Timeout(self.timeout)),
            }
            if attempt < self.retries {
                UPLOAD_RETRIES.fetch_add(1, Ordering::Relaxed);
                let backoff = Duration::from_millis(100 * (1u64 << attempt.min(5)));
                tokio::time::sleep(backoff).await;
            }
        }
        UPLOAD_FAILURE.fetch_add(1, Ordering::Relaxed);
        Err(last_error.unwrap_or_else(|| ArchiveError::S3("upload failed".to_string())))
    }

    /// Create the media bucket when missing. Safe to call on every startup.
    pub async fn ensure_bucket(&self) -> Result<(), ArchiveError> {
        if self
            .client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
        {
            return Ok(());
        }
        self.client
            .create_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(|error| ArchiveError::S3(error.to_string()))?;
        Ok(())
    }

    pub async fn put(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), ArchiveError> {
        self.validate_key(key)?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|error| ArchiveError::S3(error.to_string()))?;
        Ok(())
    }

    pub async fn list_prefix(&self, prefix: &str) -> Result<Vec<ArchivedObject>, ArchiveError> {
        let mut objects = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut request = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix)
                .max_keys(1000);
            if let Some(token) = token.as_deref() {
                request = request.continuation_token(token);
            }
            let output = request
                .send()
                .await
                .map_err(|error| ArchiveError::S3(error.to_string()))?;
            objects.extend(output.contents().iter().filter_map(|object| {
                Some(ArchivedObject {
                    key: object.key()?.to_string(),
                    size: object.size().unwrap_or_default(),
                    modified_epoch: object.last_modified().map(|value| value.secs()),
                })
            }));
            if !output.is_truncated().unwrap_or(false) {
                break;
            }
            token = output.next_continuation_token().map(str::to_string);
            if token.is_none() {
                break;
            }
        }
        Ok(objects)
    }

    pub async fn delete(&self, key: &str) -> Result<(), ArchiveError> {
        self.validate_key(key)?;
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|error| ArchiveError::S3(error.to_string()))?;
        Ok(())
    }

    /// Delete objects older than `retention_seconds`; returns the delete count.
    pub async fn cleanup_expired(&self, retention_seconds: i64) -> Result<usize, ArchiveError> {
        if retention_seconds <= 0 {
            return Ok(0);
        }
        let cutoff = Utc::now().timestamp() - retention_seconds;
        let prefix = format!("{}/", self.prefix);
        let mut deleted = 0;
        for object in self.list_prefix(&prefix).await? {
            let expired = object
                .modified_epoch
                .map(|modified| modified < cutoff)
                .unwrap_or(false);
            if expired {
                self.delete(&object.key).await?;
                deleted += 1;
                CLEANUP_DELETED.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(deleted)
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>, ArchiveError> {
        self.validate_key(key)?;
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|error| ArchiveError::S3(error.to_string()))?;
        let data = output
            .body
            .collect()
            .await
            .map_err(|error| ArchiveError::S3(error.to_string()))?;
        Ok(data.into_bytes().to_vec())
    }

    pub fn validate_key(&self, key: &str) -> Result<(), ArchiveError> {
        let prefix = format!("{}/", self.prefix);
        if !key.starts_with(&prefix)
            || key.contains("..")
            || key.contains('\\')
            || key.ends_with('/')
        {
            return Err(ArchiveError::InvalidKey(key.to_string()));
        }
        Ok(())
    }
}

pub fn global() -> Option<&'static RustFsArchive> {
    static ARCHIVE: OnceLock<Option<RustFsArchive>> = OnceLock::new();
    ARCHIVE.get_or_init(RustFsArchive::from_env).as_ref()
}

pub fn enabled() -> bool {
    matches!(
        std::env::var("TSP_RUSTFS_ARCHIVE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn segment_seconds() -> u64 {
    std::env::var("TSP_RUSTFS_SEGMENT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &u64| *value >= 1)
        .unwrap_or(30)
}

pub fn segment_max_bytes() -> usize {
    std::env::var("TSP_RUSTFS_SEGMENT_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &usize| *value >= 1024 * 1024)
        .unwrap_or(64 * 1024 * 1024)
}

pub fn segment_bytes() -> usize {
    std::env::var("TSP_RUSTFS_SEGMENT_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &usize| *value >= 64 * 1024)
        .unwrap_or(8 * 1024 * 1024)
}

pub fn retention_days() -> u32 {
    env_u32("TSP_RUSTFS_RETENTION_DAYS", 0, 0, 3650)
}

pub fn cleanup_interval_seconds() -> u64 {
    env_u32("TSP_RUSTFS_CLEANUP_INTERVAL_SECS", 3600, 60, 86_400) as u64
}

fn env_u32(name: &str, default: u32, min: u32, max: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn normalize_prefix(prefix: &str) -> String {
    prefix.trim().trim_matches('/').to_string()
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive() -> RustFsArchive {
        RustFsArchive::from_parts(
            "http://127.0.0.1:9000",
            "tsp-media",
            "access",
            "secret",
            "us-east-1",
            "/media/",
        )
        .expect("archive")
    }

    #[test]
    fn media_keys_are_prefixed_and_path_safe() {
        let archive = archive();
        assert_eq!(archive.prefix(), "media");
        let at = DateTime::parse_from_rfc3339("2026-11-24T08:00:01Z")
            .unwrap()
            .with_timezone(&Utc);
        let key = archive.media_key("138000000001", 2, at, 7, "h264");
        assert_eq!(key, "media/138000000001/2/2026/11/24/080001-000007.h264");
        assert!(archive
            .validate_key("media/138000000001/2/2026/11/24/080001-000007.h264")
            .is_ok());
        assert!(archive.validate_key("other/file.h264").is_err());
        assert!(archive.validate_key("media/../secret").is_err());
    }

    #[test]
    fn prefix_and_segment_limits_have_safe_defaults() {
        assert_eq!(normalize_prefix("/a/b/"), "a/b");
        assert!(segment_seconds() >= 1);
        assert!(segment_bytes() >= 64 * 1024);
    }

    #[tokio::test]
    async fn archive_put_list_get_against_s3_mock() {
        use std::{
            collections::HashMap,
            sync::{Arc, Mutex},
        };

        use axum::{
            body::Bytes,
            extract::{Path, State},
            http::StatusCode,
            routing::get,
            Router,
        };

        type Objects = Arc<Mutex<HashMap<String, Vec<u8>>>>;

        async fn put_object(
            State(objects): State<Objects>,
            Path(key): Path<String>,
            body: Bytes,
        ) -> StatusCode {
            objects.lock().unwrap().insert(key, body.to_vec());
            StatusCode::OK
        }

        async fn get_object(
            State(objects): State<Objects>,
            Path(key): Path<String>,
        ) -> Result<Vec<u8>, StatusCode> {
            objects
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .ok_or(StatusCode::NOT_FOUND)
        }

        async fn list_objects(
            State(objects): State<Objects>,
        ) -> ([(&'static str, &'static str); 1], String) {
            let objects = objects.lock().unwrap();
            let contents: String = objects
                .keys()
                .map(|key| {
                    format!(
                        "<Contents><Key>{key}</Key>\
                         <LastModified>2026-11-24T08:00:00.000Z</LastModified>\
                         <ETag>\"etag\"</ETag><Size>{}</Size>\
                         <StorageClass>STANDARD</StorageClass></Contents>",
                        objects[key].len()
                    )
                })
                .collect();
            let xml = format!(
                "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
                 <Name>tsp-media</Name><Prefix></Prefix><KeyCount>{}</KeyCount>\
                 <MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>",
                objects.len()
            );
            ([("content-type", "application/xml")], xml)
        }

        let objects: Objects = Arc::new(Mutex::new(HashMap::new()));
        let app = Router::new()
            .route("/tsp-media", get(list_objects).with_state(objects.clone()))
            .route("/tsp-media/", get(list_objects).with_state(objects.clone()))
            .route(
                "/tsp-media/{*key}",
                get(get_object).put(put_object).with_state(objects.clone()),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let archive = RustFsArchive::from_parts(
            &format!("http://{addr}"),
            "tsp-media",
            "access",
            "secret",
            "us-east-1",
            "media",
        )
        .expect("archive");
        let key = "media/138000000001/2/2026/11/24/080001-000000.h264";
        archive
            .put(key, b"segment".to_vec(), "video/h264")
            .await
            .expect("put");
        let listed = archive
            .list_prefix("media/138000000001/2/")
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, key);
        assert_eq!(listed[0].size, 7);
        let bytes = archive.get(key).await.expect("get");
        assert_eq!(bytes, b"segment");
    }

    #[tokio::test]
    async fn upload_retries_transient_s3_failures() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use axum::{
            body::Bytes,
            extract::{Path, State},
            http::StatusCode,
            routing::put,
            Router,
        };

        #[derive(Clone)]
        struct RetryState {
            objects: Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>,
            puts: Arc<AtomicUsize>,
        }

        async fn flaky_put(
            State(state): State<RetryState>,
            Path(key): Path<String>,
            body: Bytes,
        ) -> StatusCode {
            let attempt = state.puts.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
            state.objects.lock().unwrap().insert(key, body.to_vec());
            StatusCode::OK
        }

        let state = RetryState {
            objects: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            puts: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new().route(
            "/tsp-media/{*key}",
            put(flaky_put).with_state(state.clone()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let archive = RustFsArchive::from_parts(
            &format!("http://{addr}"),
            "tsp-media",
            "access",
            "secret",
            "us-east-1",
            "media",
        )
        .expect("archive")
        .with_upload_policy(2, Duration::from_secs(2));
        archive
            .put_with_retry("media/1/1/test.h264", b"payload".to_vec(), "video/h264")
            .await
            .expect("retry succeeds");
        assert_eq!(state.puts.load(Ordering::SeqCst), 3);
        assert_eq!(state.objects.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cleanup_deletes_only_expired_objects() {
        use axum::{
            extract::{Path, State},
            http::StatusCode,
            routing::{delete, get},
            Router,
        };

        type Objects = Arc<std::sync::Mutex<std::collections::HashMap<String, (Vec<u8>, i64)>>>;

        async fn list_objects(State(objects): State<Objects>) -> String {
            let objects = objects.lock().unwrap();
            let contents: String = objects
                .iter()
                .map(|(key, (bytes, epoch))| {
                    let stamp = chrono::DateTime::<chrono::Utc>::from_timestamp(*epoch, 0)
                        .unwrap()
                        .format("%Y-%m-%dT%H:%M:%S.000Z");
                    format!(
                        "<Contents><Key>{key}</Key><LastModified>{stamp}</LastModified>\
                         <ETag>\"etag\"</ETag><Size>{}</Size>\
                         <StorageClass>STANDARD</StorageClass></Contents>",
                        bytes.len()
                    )
                })
                .collect();
            format!(
                "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
                 <Name>tsp-media</Name><Prefix></Prefix><KeyCount>{}</KeyCount>\
                 <MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>",
                objects.len()
            )
        }

        async fn delete_object(
            State(objects): State<Objects>,
            Path(key): Path<String>,
        ) -> StatusCode {
            objects.lock().unwrap().remove(&key);
            StatusCode::NO_CONTENT
        }

        let now = chrono::Utc::now().timestamp();
        let objects: Objects = Arc::new(std::sync::Mutex::new(
            [
                ("media/1/1/old.h264".to_string(), (vec![0u8; 3], now - 7200)),
                ("media/1/1/new.h264".to_string(), (vec![0u8; 3], now - 60)),
            ]
            .into_iter()
            .collect(),
        ));
        let app = Router::new()
            .route("/tsp-media/", get(list_objects).with_state(objects.clone()))
            .route(
                "/tsp-media/{*key}",
                delete(delete_object).with_state(objects.clone()),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let archive = RustFsArchive::from_parts(
            &format!("http://{addr}"),
            "tsp-media",
            "access",
            "secret",
            "us-east-1",
            "media",
        )
        .expect("archive");
        assert_eq!(archive.cleanup_expired(3600).await.expect("cleanup"), 1);
        let left = objects.lock().unwrap();
        assert_eq!(left.len(), 1);
        assert!(left.contains_key("media/1/1/new.h264"));
    }
}
