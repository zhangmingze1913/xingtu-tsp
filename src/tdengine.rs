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

use std::sync::{
    atomic::{AtomicI64, AtomicU64, Ordering},
    OnceLock,
};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
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
pub static ALERTS_SENT: AtomicU64 = AtomicU64::new(0);
pub static ALERTS_FAILED: AtomicU64 = AtomicU64::new(0);
pub static ALERTS_SKIPPED: AtomicU64 = AtomicU64::new(0);
pub static HISTORY_PRUNED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_JOBS_STARTED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_JOBS_COMPLETED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_JOBS_FAILED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_JOBS_CANCELLED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_JOBS_RETRIED: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default)]
pub struct TdengineStats {
    pub writes: u64,
    pub failures: u64,
    pub retries: u64,
    pub dropped: u64,
    pub queue_depth: i64,
    pub reconcile_checks: u64,
    pub reconcile_mismatches: u64,
    pub alerts_sent: u64,
    pub alerts_failed: u64,
    pub alerts_skipped: u64,
    pub history_pruned: u64,
    pub export_jobs_started: u64,
    pub export_jobs_completed: u64,
    pub export_jobs_failed: u64,
    pub export_jobs_cancelled: u64,
    pub export_jobs_retried: u64,
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
        alerts_sent: ALERTS_SENT.load(Ordering::Relaxed),
        alerts_failed: ALERTS_FAILED.load(Ordering::Relaxed),
        alerts_skipped: ALERTS_SKIPPED.load(Ordering::Relaxed),
        history_pruned: HISTORY_PRUNED.load(Ordering::Relaxed),
        export_jobs_started: EXPORT_JOBS_STARTED.load(Ordering::Relaxed),
        export_jobs_completed: EXPORT_JOBS_COMPLETED.load(Ordering::Relaxed),
        export_jobs_failed: EXPORT_JOBS_FAILED.load(Ordering::Relaxed),
        export_jobs_cancelled: EXPORT_JOBS_CANCELLED.load(Ordering::Relaxed),
        export_jobs_retried: EXPORT_JOBS_RETRIED.load(Ordering::Relaxed),
    }
}

#[derive(Default)]
struct AlertCooldown {
    last: std::sync::Mutex<std::collections::HashMap<String, i64>>,
}

impl AlertCooldown {
    fn allow_at(&self, key: &str, now: i64, cooldown: i64) -> bool {
        let Ok(mut last) = self.last.lock() else {
            return true;
        };
        if last.len() > 1024 {
            last.retain(|_, at| now - *at < cooldown);
        }
        if let Some(previous) = last.get(key) {
            if now - *previous < cooldown {
                return false;
            }
        }
        last.insert(key.to_string(), now);
        true
    }
}

static ALERT_COOLDOWN: OnceLock<AlertCooldown> = OnceLock::new();

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

    /// Lightweight liveness query used by health/readiness probes.
    pub async fn ping(&self) -> AppResult<()> {
        self.exec("SELECT SERVER_VERSION()").await.map(|_| ())
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

/// Send one reconciliation mismatch alert. A global cooldown prevents a
/// scheduled run from flooding the webhook once per vehicle.
pub async fn notify_reconcile_mismatch(
    vehicle_id: &str,
    from: &str,
    to: &str,
    pg_count: i64,
    td_count: i64,
    delta: i64,
) {
    let Ok(url) = std::env::var("TSP_RECONCILE_ALERT_WEBHOOK") else {
        return;
    };
    if url.trim().is_empty() {
        return;
    }
    let cooldown = std::env::var("TSP_RECONCILE_ALERT_COOLDOWN_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(300)
        .max(1);
    let scope = std::env::var("TSP_RECONCILE_ALERT_COOLDOWN_SCOPE")
        .unwrap_or_else(|_| "global".to_string());
    let key = if scope.eq_ignore_ascii_case("vehicle") {
        vehicle_id
    } else {
        "*"
    };
    let now = Utc::now().timestamp();
    if !ALERT_COOLDOWN
        .get_or_init(AlertCooldown::default)
        .allow_at(key, now, cooldown)
    {
        ALERTS_SKIPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let format =
        std::env::var("TSP_RECONCILE_ALERT_FORMAT").unwrap_or_else(|_| "generic".to_string());
    let payload = alert_payload(&format, vehicle_id, from, to, pg_count, td_count, delta);
    match send_webhook(url.trim(), &payload).await {
        Ok(()) => {
            ALERTS_SENT.fetch_add(1, Ordering::Relaxed);
        }
        Err(error) => {
            ALERTS_FAILED.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(%error, %url, "reconciliation alert webhook failed");
        }
    }
}

/// Send one aggregated alert for a scheduled reconciliation run.
pub async fn notify_reconcile_summary(vehicles: &[String], from: &str, to: &str) {
    let Ok(url) = std::env::var("TSP_RECONCILE_ALERT_WEBHOOK") else {
        return;
    };
    if url.trim().is_empty() {
        return;
    }
    let cooldown = std::env::var("TSP_RECONCILE_ALERT_COOLDOWN_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(300)
        .max(1);
    let now = Utc::now().timestamp();
    if !ALERT_COOLDOWN
        .get_or_init(AlertCooldown::default)
        .allow_at("*summary", now, cooldown)
    {
        ALERTS_SKIPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let format =
        std::env::var("TSP_RECONCILE_ALERT_FORMAT").unwrap_or_else(|_| "generic".to_string());
    let payload = summary_alert_payload(&format, vehicles, from, to);
    match send_webhook(url.trim(), &payload).await {
        Ok(()) => {
            ALERTS_SENT.fetch_add(1, Ordering::Relaxed);
        }
        Err(error) => {
            ALERTS_FAILED.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(%error, %url, "reconciliation summary alert failed");
        }
    }
}

pub(crate) async fn send_webhook(url: &str, payload: &Value) -> AppResult<()> {
    let secret = std::env::var("TSP_RECONCILE_ALERT_WEBHOOK_SECRET").ok();
    send_webhook_signed(url, payload, secret.as_deref()).await
}

pub(crate) async fn send_webhook_signed(
    url: &str,
    payload: &Value,
    secret: Option<&str>,
) -> AppResult<()> {
    let body = serde_json::to_vec(payload)
        .map_err(|error| AppError::internal(format!("alert payload: {error}")))?;
    let signature = secret.map(|secret| webhook_signature(secret, &body));
    let retries = std::env::var("TSP_RECONCILE_ALERT_RETRIES")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(2)
        .min(5);
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|error| AppError::internal(format!("alert client: {error}")))?;
    let mut last_error = String::new();
    for attempt in 0..=retries {
        let mut request = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.clone());
        if let Some(signature) = signature.as_deref() {
            request = request.header("X-TSP-Signature", signature);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => last_error = format!("alert webhook HTTP {}", response.status()),
            Err(error) => last_error = format!("alert request: {error}"),
        }
        if attempt < retries {
            let backoff = std::time::Duration::from_millis(100 * (1u64 << attempt.min(5)));
            tokio::time::sleep(backoff).await;
        }
    }
    Err(AppError::internal(last_error))
}

pub(crate) fn summary_alert_payload(
    format: &str,
    vehicles: &[String],
    from: &str,
    to: &str,
) -> Value {
    let count = vehicles.len();
    let sample: Vec<&String> = vehicles.iter().take(20).collect();
    let checked_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if format.eq_ignore_ascii_case("alertmanager") {
        serde_json::json!([{
            "labels": {
                "alertname": "TdengineReconcileMismatchSummary",
                "severity": "warning",
                "service": "xingtu-tsp",
            },
            "annotations": {
                "summary": "TDengine/PostgreSQL reconciliation mismatches",
                "description": format!(
                    "mismatch_vehicles={count} window={from}..{to} sample={sample:?}"
                ),
            },
            "startsAt": checked_at,
        }])
    } else {
        serde_json::json!({
            "type": "tdengine_reconcile_summary",
            "mismatch_vehicles": count,
            "vehicles": sample,
            "from": from,
            "to": to,
            "checked_at": checked_at,
        })
    }
}

pub(crate) fn alert_payload(
    format: &str,
    vehicle_id: &str,
    from: &str,
    to: &str,
    pg_count: i64,
    td_count: i64,
    delta: i64,
) -> Value {
    let checked_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if format.eq_ignore_ascii_case("alertmanager") {
        serde_json::json!([{
            "labels": {
                "alertname": "TdengineReconcileMismatch",
                "severity": "warning",
                "service": "xingtu-tsp",
                "vehicle_id": vehicle_id,
            },
            "annotations": {
                "summary": "TDengine/PostgreSQL reconciliation mismatch",
                "description": format!(
                    "vehicle={vehicle_id} pg={pg_count} tdengine={td_count} delta={delta} window={from}..{to}"
                ),
            },
            "startsAt": checked_at,
        }])
    } else {
        serde_json::json!({
            "type": "tdengine_reconcile_mismatch",
            "vehicle_id": vehicle_id,
            "from": from,
            "to": to,
            "pg_count": pg_count,
            "tdengine_count": td_count,
            "count_delta": delta,
            "checked_at": checked_at,
        })
    }
}

pub(crate) fn webhook_signature(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    format!(
        "sha256={}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
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

    #[tokio::test]
    async fn webhook_posts_json_payload() {
        use std::sync::{Arc, Mutex};

        use axum::{extract::State, routing::post, Json, Router};

        type Seen = Arc<Mutex<Vec<serde_json::Value>>>;

        async fn alert(State(seen): State<Seen>, Json(value): Json<serde_json::Value>) {
            seen.lock().unwrap().push(value);
        }

        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().route("/alert", post(alert).with_state(seen.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        send_webhook(
            &format!("http://{addr}/alert"),
            &serde_json::json!({"type": "tdengine_reconcile_mismatch", "delta": 2}),
        )
        .await
        .expect("webhook");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["delta"], 2);
    }

    #[tokio::test]
    async fn webhook_signs_and_retries() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
            Arc, Mutex,
        };

        use axum::{
            body::Bytes,
            extract::State,
            http::{HeaderMap, StatusCode},
            routing::post,
            Router,
        };

        type LastSeen = Option<(Option<String>, Vec<u8>)>;

        #[derive(Clone)]
        struct StateData {
            attempts: Arc<AtomicUsize>,
            last: Arc<Mutex<LastSeen>>,
        }

        async fn alert(
            State(state): State<StateData>,
            headers: HeaderMap,
            body: Bytes,
        ) -> StatusCode {
            let attempt = state.attempts.fetch_add(1, AtomicOrdering::SeqCst);
            let signature = headers
                .get("x-tsp-signature")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            *state.last.lock().unwrap() = Some((signature, body.to_vec()));
            if attempt < 2 {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::OK
            }
        }

        let state = StateData {
            attempts: Arc::new(AtomicUsize::new(0)),
            last: Arc::new(Mutex::new(None)),
        };
        let app = Router::new().route("/alert", post(alert).with_state(state.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let payload = serde_json::json!({"type": "tdengine_reconcile_mismatch", "delta": 3});
        send_webhook_signed(&format!("http://{addr}/alert"), &payload, Some("s3cret"))
            .await
            .expect("webhook");
        assert_eq!(state.attempts.load(AtomicOrdering::SeqCst), 3);
        let (signature, body) = state.last.lock().unwrap().clone().expect("seen");
        let expected = webhook_signature("s3cret", &body);
        assert_eq!(signature.as_deref(), Some(expected.as_str()));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["delta"],
            3
        );
    }

    #[test]
    fn alert_payload_supports_generic_and_alertmanager() {
        let generic = alert_payload("generic", "v1", "a", "b", 10, 8, 2);
        assert_eq!(generic["type"], "tdengine_reconcile_mismatch");
        assert_eq!(generic["count_delta"], 2);
        let am = alert_payload("alertmanager", "v1", "a", "b", 10, 8, 2);
        let item = &am.as_array().expect("array")[0];
        assert_eq!(item["labels"]["alertname"], "TdengineReconcileMismatch");
        assert_eq!(item["labels"]["vehicle_id"], "v1");
        assert!(item["annotations"]["description"]
            .as_str()
            .unwrap()
            .contains("delta=2"));
    }

    #[test]
    fn alert_cooldown_respects_scope_and_window() {
        let cooldown = AlertCooldown::default();
        assert!(cooldown.allow_at("v1", 100, 300));
        assert!(!cooldown.allow_at("v1", 101, 300));
        assert!(cooldown.allow_at("v2", 101, 300));
        assert!(cooldown.allow_at("v1", 401, 300));
        assert!(cooldown.allow_at("*", 100, 300));
        assert!(!cooldown.allow_at("*", 200, 300));
    }

    #[test]
    fn summary_payload_supports_both_formats() {
        let vehicles = vec!["v1".to_string(), "v2".to_string()];
        let generic = summary_alert_payload("generic", &vehicles, "a", "b");
        assert_eq!(generic["type"], "tdengine_reconcile_summary");
        assert_eq!(generic["mismatch_vehicles"], 2);
        let am = summary_alert_payload("alertmanager", &vehicles, "a", "b");
        assert_eq!(
            am.as_array().expect("array")[0]["labels"]["alertname"],
            "TdengineReconcileMismatchSummary"
        );
    }
}
