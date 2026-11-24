//! Optional TDengine time-series adapter.
//!
//! TDengine 3.x exposes a REST SQL endpoint through taosAdapter (`:6041`).
//! Telemetry is written to a super table (`telemetry`) with one child table per
//! vehicle, and trajectories are queried with a tag filter.
//!
//! Configuration:
//!   TSP_TDENGINE_URL=http://tdengine:6041
//!   TSP_TDENGINE_USER=root
//!   TSP_TDENGINE_PASSWORD=taosdata
//!   TSP_TDENGINE_DATABASE=tsp

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    error::{AppError, AppResult},
    models::{IngestOutcome, TelemetryPoint},
};

pub static WRITES: AtomicU64 = AtomicU64::new(0);
pub static FAILURES: AtomicU64 = AtomicU64::new(0);
pub static RETRIES: AtomicU64 = AtomicU64::new(0);
pub static DROPPED: AtomicU64 = AtomicU64::new(0);
pub static QUEUE_DEPTH: AtomicI64 = AtomicI64::new(0);
pub static RECONCILE_CHECKS: AtomicU64 = AtomicU64::new(0);
pub static RECONCILE_MISMATCHES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default)]
pub struct TdengineStats {
    pub writes: u64,
    pub failures: u64,
    pub retries: u64,
    pub dropped: u64,
    pub queue_depth: i64,
    pub reconcile_checks: u64,
    pub reconcile_mismatches: u64,
}

pub fn stats() -> TdengineStats {
    TdengineStats {
        writes: WRITES.load(Ordering::Relaxed),
        failures: FAILURES.load(Ordering::Relaxed),
        retries: RETRIES.load(Ordering::Relaxed),
        dropped: DROPPED.load(Ordering::Relaxed),
        queue_depth: QUEUE_DEPTH.load(Ordering::Relaxed),
        reconcile_checks: RECONCILE_CHECKS.load(Ordering::Relaxed),
        reconcile_mismatches: RECONCILE_MISMATCHES.load(Ordering::Relaxed),
    }
}

#[derive(Clone)]
pub struct TdengineStore {
    sender: mpsc::Sender<Sample>,
    client: Client,
    base: String,
    user: String,
    password: String,
    database: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Sample {
    pub(crate) vehicle_id: String,
    pub(crate) ts_ms: i64,
    pub(crate) lat: f64,
    pub(crate) lon: f64,
    pub(crate) speed: f64,
    pub(crate) heading: f64,
    pub(crate) ignition: bool,
    pub(crate) soc: f64,
    pub(crate) odometer_km: f64,
}

impl TdengineStore {
    pub async fn start(
        url: String,
        user: String,
        password: String,
        database: String,
    ) -> AppResult<Self> {
        let base = url.trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(AppError::internal(
                "TSP_TDENGINE_URL 必须是 taosAdapter HTTP 地址，例如 http://tdengine:6041",
            ));
        }
        let database = sanitize_identifier(&database);
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|error| {
                tracing::error!(%error, "failed to build TDengine HTTP client");
                AppError::internal("TDengine HTTP 客户端初始化失败")
            })?;

        let capacity = std::env::var("TSP_TDENGINE_QUEUE_CAPACITY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value >= 64)
            .unwrap_or(8192);
        let (sender, mut receiver) = mpsc::channel::<Sample>(capacity);
        let store = Self {
            sender,
            client,
            base,
            user,
            password,
            database,
        };
        store
            .exec(&format!(
                "CREATE DATABASE IF NOT EXISTS `{}`",
                store.database
            ))
            .await?;
        store
            .exec(&format!(
                "CREATE STABLE IF NOT EXISTS `{}`.`telemetry` (\
                 ts TIMESTAMP, lat DOUBLE, lon DOUBLE, speed DOUBLE, heading DOUBLE, \
                 ignition BOOL, soc DOUBLE, odometer DOUBLE\
                 ) TAGS (vehicle_id NCHAR(64))",
                store.database
            ))
            .await?;
        tracing::info!(url = %store.base, database = %store.database, "TDengine trajectory store enabled");

        let writer = store.clone();
        tokio::spawn(async move {
            while let Some(sample) = receiver.recv().await {
                QUEUE_DEPTH.fetch_sub(1, Ordering::Relaxed);
                writer.write_sample_with_retry(&sample).await;
            }
        });
        if crate::tdengine_spool::global().is_some() {
            let replay = store.clone();
            tokio::spawn(async move {
                let interval = std::time::Duration::from_secs(
                    crate::tdengine_spool::replay_interval_seconds(),
                );
                let mut ticker = tokio::time::interval(interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    ticker.tick().await;
                    let replayed = replay.replay_spool().await;
                    if replayed > 0 {
                        tracing::info!(replayed, "TDengine spool replay completed");
                    }
                }
            });
        }
        Ok(store)
    }

    pub fn enqueue(&self, outcome: &IngestOutcome) -> bool {
        let Ok(ts) = DateTime::parse_from_rfc3339(&outcome.ts) else {
            return false;
        };
        let sample = Sample {
            vehicle_id: outcome.vehicle_id.clone(),
            ts_ms: ts.timestamp_millis(),
            lat: outcome.lat,
            lon: outcome.lon,
            speed: outcome.speed,
            heading: outcome.heading,
            ignition: outcome.ignition,
            soc: outcome.soc,
            odometer_km: outcome.odometer_km,
        };
        match self.sender.try_send(sample) {
            Ok(()) => {
                QUEUE_DEPTH.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(_) => {
                DROPPED.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(vehicle = %outcome.vehicle_id, "TDengine queue full; sample dropped");
                false
            }
        }
    }

    pub async fn query(
        &self,
        vehicle_id: &str,
        from: &str,
        to: &str,
        limit: i64,
    ) -> AppResult<Vec<TelemetryPoint>> {
        let start = DateTime::parse_from_rfc3339(from)
            .map_err(|_| AppError::bad_request("from 必须是 RFC3339 时间"))?
            .timestamp_millis();
        let end = DateTime::parse_from_rfc3339(to)
            .map_err(|_| AppError::bad_request("to 必须是 RFC3339 时间"))?
            .timestamp_millis();
        let limit = limit.clamp(1, 2000);
        let id = escape_sql(vehicle_id);
        let sql = format!(
            "SELECT ts, lat, lon, speed, heading, ignition, soc, odometer \
             FROM `{db}`.`telemetry` WHERE vehicle_id = '{id}' \
             AND ts >= {start} AND ts <= {end} ORDER BY ts ASC LIMIT {limit}",
            db = self.database,
            id = id,
            start = start,
            end = end,
            limit = limit
        );
        let value = self.exec(&sql).await?;
        let Some(rows) = value.get("data").and_then(Value::as_array) else {
            return Ok(Vec::new());
        };
        let mut points = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(cells) = row.as_array() else {
                continue;
            };
            if cells.len() < 8 {
                continue;
            }
            let ts_ms = parse_timestamp(&cells[0]);
            let Some(ts_ms) = ts_ms else {
                continue;
            };
            points.push(TelemetryPoint {
                id: 0,
                vehicle_id: vehicle_id.to_string(),
                ts: DateTime::<Utc>::from_timestamp_millis(ts_ms)
                    .unwrap_or_else(Utc::now)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                lat: number(&cells[1]),
                lon: number(&cells[2]),
                speed: number(&cells[3]),
                heading: number(&cells[4]),
                ignition: cells[5].as_bool().unwrap_or(true),
                soc: number(&cells[6]),
                odometer_km: number(&cells[7]),
                source: "tdengine".to_string(),
            });
        }
        Ok(points)
    }

    pub async fn range_stats(
        &self,
        vehicle_id: &str,
        from: &str,
        to: &str,
    ) -> AppResult<crate::models::TelemetryRangeStats> {
        let start = DateTime::parse_from_rfc3339(from)
            .map_err(|_| AppError::bad_request("from 必须是 RFC3339 时间"))?
            .timestamp_millis();
        let end = DateTime::parse_from_rfc3339(to)
            .map_err(|_| AppError::bad_request("to 必须是 RFC3339 时间"))?
            .timestamp_millis();
        let id = escape_sql(vehicle_id);
        let sql = format!(
            "SELECT COUNT(*), MIN(ts), MAX(ts) FROM `{db}`.`telemetry` \
             WHERE vehicle_id = '{id}' AND ts >= {start} AND ts <= {end}",
            db = self.database,
            id = id,
            start = start,
            end = end
        );
        let value = self.exec(&sql).await?;
        let Some(row) = value
            .get("data")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(Value::as_array)
        else {
            return Ok(crate::models::TelemetryRangeStats {
                count: 0,
                min_ts: None,
                max_ts: None,
            });
        };
        let count = row
            .first()
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            })
            .unwrap_or(0);
        Ok(crate::models::TelemetryRangeStats {
            count,
            min_ts: row.get(1).and_then(timestamp_text),
            max_ts: row.get(2).and_then(timestamp_text),
        })
    }

    async fn write_with_retry(&self, sample: &Sample) -> bool {
        let retries = std::env::var("TSP_TDENGINE_WRITE_RETRIES")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(3)
            .min(10);
        for attempt in 0..=retries {
            match self.write_sample(sample).await {
                Ok(()) => {
                    WRITES.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
                Err(error) => {
                    if attempt < retries {
                        RETRIES.fetch_add(1, Ordering::Relaxed);
                        let backoff =
                            std::time::Duration::from_millis(100 * (1u64 << attempt.min(5)));
                        tokio::time::sleep(backoff).await;
                    } else {
                        FAILURES.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(%error, vehicle = %sample.vehicle_id, "TDengine write failed after retries");
                    }
                }
            }
        }
        false
    }

    async fn write_sample_with_retry(&self, sample: &Sample) {
        if self.write_with_retry(sample).await {
            return;
        }
        if let Some(spool) = crate::tdengine_spool::global() {
            match spool.save(sample).await {
                Ok(()) => tracing::warn!(vehicle = %sample.vehicle_id, "TDengine sample spooled"),
                Err(error) => {
                    crate::tdengine_spool::SPOOL_FAILED.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(%error, vehicle = %sample.vehicle_id, "TDengine spool write failed");
                }
            }
        }
    }

    pub async fn replay_spool(&self) -> usize {
        let Some(spool) = crate::tdengine_spool::global() else {
            return 0;
        };
        let Ok(items) = spool.pending().await else {
            return 0;
        };
        let mut replayed = 0;
        for (id, sample) in items {
            if self.write_with_retry(&sample).await && spool.remove(&id).await.is_ok() {
                crate::tdengine_spool::REPLAYED.fetch_add(1, Ordering::Relaxed);
                replayed += 1;
            }
        }
        replayed
    }

    async fn write_sample(&self, sample: &Sample) -> AppResult<()> {
        let table = sanitize_identifier(&format!("v_{}", sample.vehicle_id));
        let sql = format!(
            "INSERT INTO `{db}`.`{table}` USING `{db}`.`telemetry` TAGS ('{id}') \
             VALUES ({ts}, {lat}, {lon}, {speed}, {heading}, {ignition}, {soc}, {odo})",
            db = self.database,
            table = table,
            id = escape_sql(&sample.vehicle_id),
            ts = sample.ts_ms,
            lat = sample.lat,
            lon = sample.lon,
            speed = sample.speed,
            heading = sample.heading,
            ignition = if sample.ignition { "true" } else { "false" },
            soc = sample.soc,
            odo = sample.odometer_km
        );
        self.exec(&sql).await.map(|_| ())
    }

    async fn exec(&self, sql: &str) -> AppResult<Value> {
        let response = self
            .client
            .post(format!("{}/rest/sql", self.base))
            .basic_auth(&self.user, Some(&self.password))
            .header("Content-Type", "text/plain")
            .body(sql.to_string())
            .send()
            .await
            .map_err(|error| {
                tracing::warn!(%error, "TDengine REST request failed");
                AppError::internal("TDengine 请求失败")
            })?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "TDengine HTTP {}",
                response.status()
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|_| AppError::internal("TDengine 响应解析失败"))?;
        let code = value.get("code").and_then(Value::as_i64).unwrap_or(0);
        if code != 0 {
            let desc = value
                .get("desc")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(AppError::internal(format!("TDengine 错误 {code}: {desc}")));
        }
        Ok(value)
    }
}

fn timestamp_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let ms = value.as_i64()?;
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|ts| ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    if let Some(ms) = value.as_i64() {
        return Some(if ms < 10_000_000_000 { ms * 1000 } else { ms });
    }
    let text = value.as_str()?;
    if let Ok(ms) = text.parse::<i64>() {
        return Some(if ms < 10_000_000_000 { ms * 1000 } else { ms });
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|ts| ts.timestamp_millis())
}

fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
        .unwrap_or(0.0)
}

fn escape_sql(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "''")
}

fn sanitize_identifier(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "tsp".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamps_and_identifiers() {
        assert_eq!(
            parse_timestamp(&serde_json::json!(1_700_000_000)),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_timestamp(&serde_json::json!(1_700_000_000_000i64)),
            Some(1_700_000_000_000)
        );
        let text = "2026-11-24T08:00:00Z";
        let expected = DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp_millis();
        assert_eq!(parse_timestamp(&serde_json::json!(text)), Some(expected));
        assert_eq!(sanitize_identifier("v_沪A 123"), "v__A_123");
        assert_eq!(escape_sql("a'b\\c"), "a''b\\\\c");
    }

    #[tokio::test]
    async fn rest_query_maps_rows_to_points() {
        use axum::{routing::post, Router};

        async fn sql(body: String) -> axum::Json<serde_json::Value> {
            if body.trim_start().starts_with("SELECT") {
                axum::Json(serde_json::json!({
                    "code": 0,
                    "data": [[1_700_000_000_000i64, 31.2, 121.4, 50.0, 90.0, true, 80.0, 100.0]]
                }))
            } else {
                axum::Json(serde_json::json!({"code": 0}))
            }
        }

        let app = Router::new().route("/rest/sql", post(sql));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let store = TdengineStore::start(
            format!("http://{addr}"),
            "root".to_string(),
            "taosdata".to_string(),
            "tsp".to_string(),
        )
        .await
        .expect("store");
        let points = store
            .query("veh-1", "2026-11-24T00:00:00Z", "2026-11-25T00:00:00Z", 10)
            .await
            .expect("query");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].source, "tdengine");
        assert!((points[0].lat - 31.2).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn write_retries_transient_failures() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
            Arc,
        };

        use axum::{extract::State, routing::post, Router};

        async fn sql(
            State(inserts): State<Arc<AtomicUsize>>,
            body: String,
        ) -> axum::Json<serde_json::Value> {
            if body.trim_start().starts_with("INSERT") {
                let attempt = inserts.fetch_add(1, AtomicOrdering::SeqCst);
                if attempt < 2 {
                    return axum::Json(serde_json::json!({"code": 1, "desc": "busy"}));
                }
            }
            axum::Json(serde_json::json!({"code": 0}))
        }

        let inserts = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route("/rest/sql", post(sql).with_state(inserts.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let store = TdengineStore::start(
            format!("http://{addr}"),
            "root".to_string(),
            "taosdata".to_string(),
            "tsp".to_string(),
        )
        .await
        .expect("store");
        let before = stats();
        assert!(store.enqueue(&IngestOutcome {
            vehicle_id: "veh-tdengine".to_string(),
            plate_no: "沪A12345".to_string(),
            ts: "2026-11-24T08:00:00Z".to_string(),
            lat: 31.2,
            lon: 121.4,
            speed: 50.0,
            heading: 90.0,
            ignition: true,
            soc: 80.0,
            odometer_km: 100.0,
            status: "online".to_string(),
            alerts: Vec::new(),
            trip_started: false,
            trip_ended: false,
        }));
        for _ in 0..100 {
            if stats().writes > before.writes {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let after = stats();
        assert_eq!(inserts.load(AtomicOrdering::SeqCst), 3);
        assert_eq!(after.writes, before.writes + 1);
        assert!(after.retries >= before.retries + 2);
    }

    #[tokio::test]
    async fn range_stats_maps_count_and_bounds() {
        use axum::{routing::post, Router};

        async fn sql(body: String) -> axum::Json<serde_json::Value> {
            if body.contains("COUNT(*)") {
                axum::Json(serde_json::json!({
                    "code": 0,
                    "data": [[5, 1_700_000_000_000i64, 1_700_003_600_000i64]]
                }))
            } else {
                axum::Json(serde_json::json!({"code": 0}))
            }
        }

        let app = Router::new().route("/rest/sql", post(sql));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let store = TdengineStore::start(
            format!("http://{addr}"),
            "root".to_string(),
            "taosdata".to_string(),
            "tsp".to_string(),
        )
        .await
        .expect("store");
        let stats = store
            .range_stats("veh-1", "2026-11-24T00:00:00Z", "2026-11-25T00:00:00Z")
            .await
            .expect("stats");
        assert_eq!(stats.count, 5);
        assert_eq!(stats.min_ts.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(stats.max_ts.as_deref(), Some("2023-11-14T23:13:20Z"));
    }
}
