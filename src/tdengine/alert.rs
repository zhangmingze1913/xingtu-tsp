//! Reconciliation alert webhook: payloads, HMAC signing, retries and cooldown.

use std::sync::{atomic::Ordering, OnceLock};

use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::Value;
use sha2::Sha256;

use crate::error::{AppError, AppResult};

use super::{ALERTS_FAILED, ALERTS_SENT, ALERTS_SKIPPED};

#[derive(Default)]
pub(crate) struct AlertCooldown {
    last: std::sync::Mutex<std::collections::HashMap<String, i64>>,
}

impl AlertCooldown {
    pub(crate) fn allow_at(&self, key: &str, now: i64, cooldown: i64) -> bool {
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
