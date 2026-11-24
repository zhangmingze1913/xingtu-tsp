//! RustFS/S3 archive for recorded JT/T 1078 media.
//!
//! Recording is intentionally raw: H.264 Annex-B video and AAC ADTS audio are
//! uploaded as separate objects. Playback remuxes them on demand, so ingest
//! does not pay an FFmpeg cost per segment.
//!
//! Configuration is environment based; when `TSP_RUSTFS_ARCHIVE` is not truthy
//! the feature is disabled and the media hub keeps its existing in-memory-only
//! behaviour.

use std::sync::OnceLock;

use aws_sdk_s3::{
    config::{BehaviorVersion, Credentials, Region},
    primitives::ByteStream,
    Client,
};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("archive is not configured")]
    Disabled,
    #[error("invalid archive key: {0}")]
    InvalidKey(String),
    #[error("S3 request failed: {0}")]
    S3(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchivedObject {
    pub key: String,
    pub size: i64,
}

#[derive(Clone)]
pub struct RustFsArchive {
    client: Client,
    bucket: String,
    prefix: String,
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
            .build();
        Ok(Self {
            client: Client::from_conf(config),
            bucket: bucket.to_string(),
            prefix: normalize_prefix(prefix),
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
        let output = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .max_keys(1000)
            .send()
            .await
            .map_err(|error| ArchiveError::S3(error.to_string()))?;
        Ok(output
            .contents()
            .iter()
            .filter_map(|object| {
                Some(ArchivedObject {
                    key: object.key()?.to_string(),
                    size: object.size().unwrap_or_default(),
                })
            })
            .collect())
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

pub fn segment_bytes() -> usize {
    std::env::var("TSP_RUSTFS_SEGMENT_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &usize| *value >= 64 * 1024)
        .unwrap_or(8 * 1024 * 1024)
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
}
