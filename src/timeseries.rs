//! Optional tsink time-series adapter.
//!
//! tsink speaks Prometheus text import and PromQL over HTTP. Telemetry remains
//! in the relational database as the authoritative latest state; tsink is an
//! optional hot/warm trajectory store and can be queried for trajectory replay.
//!
//! Configuration:
//!   TSP_TSINK_URL=http://tsink:9201
//!   TSP_TSINK_TOKEN=optional-bearer-token

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    error::{AppError, AppResult},
    models::{IngestOutcome, TelemetryPoint},
};

#[derive(Clone)]
pub struct TsinkStore {
    sender: mpsc::UnboundedSender<Sample>,
    client: Client,
    base: String,
    token: Option<String>,
}

#[derive(Clone)]
struct Sample {
    vehicle_id: String,
    ts_ms: i64,
    lat: f64,
    lon: f64,
    speed: f64,
    heading: f64,
    ignition: bool,
    soc: f64,
    odometer_km: f64,
}

impl TsinkStore {
    pub fn start(url: String, token: Option<String>) -> AppResult<Self> {
        let base = url.trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(AppError::internal(
                "TSP_TSINK_URL 必须是 tsink HTTP 地址，例如 http://tsink:9201",
            ));
        }
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|error| {
                tracing::error!(%error, "failed to build tsink HTTP client");
                AppError::internal("tsink HTTP 客户端初始化失败")
            })?;
        let (sender, mut receiver) = mpsc::unbounded_channel::<Sample>();
        let writer_client = client.clone();
        let writer_base = base.clone();
        let writer_token = token.clone();
        tokio::spawn(async move {
            while let Some(sample) = receiver.recv().await {
                if let Err(error) = write_sample(
                    &writer_client,
                    &writer_base,
                    writer_token.as_deref(),
                    &sample,
                )
                .await
                {
                    tracing::warn!(%error, vehicle = %sample.vehicle_id, "tsink write failed");
                }
            }
        });
        tracing::info!(url = %base, "tsink trajectory store enabled");
        Ok(Self {
            sender,
            client,
            base,
            token,
        })
    }

    pub fn enqueue(&self, outcome: &IngestOutcome) {
        let Ok(ts) = DateTime::parse_from_rfc3339(&outcome.ts) else {
            return;
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
        let _ = self.sender.send(sample);
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
            .timestamp();
        let end = DateTime::parse_from_rfc3339(to)
            .map_err(|_| AppError::bad_request("to 必须是 RFC3339 时间"))?
            .timestamp();
        let mut series: BTreeMap<i64, TelemetryPoint> = BTreeMap::new();
        for (metric, field) in [
            ("tsp_lat", "lat"),
            ("tsp_lon", "lon"),
            ("tsp_speed", "speed"),
            ("tsp_heading", "heading"),
            ("tsp_soc", "soc"),
            ("tsp_odometer", "odometer_km"),
        ] {
            let values = self.query_metric(metric, vehicle_id, start, end).await?;
            for (ts_ms, value) in values {
                let point = series.entry(ts_ms).or_insert_with(|| TelemetryPoint {
                    id: 0,
                    vehicle_id: vehicle_id.to_string(),
                    ts: DateTime::<Utc>::from_timestamp_millis(ts_ms)
                        .unwrap_or_else(Utc::now)
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    lat: 0.0,
                    lon: 0.0,
                    speed: 0.0,
                    heading: 0.0,
                    ignition: true,
                    soc: 0.0,
                    odometer_km: 0.0,
                    source: "tsink".to_string(),
                });
                match field {
                    "lat" => point.lat = value,
                    "lon" => point.lon = value,
                    "speed" => point.speed = value,
                    "heading" => point.heading = value,
                    "soc" => point.soc = value,
                    "odometer_km" => point.odometer_km = value,
                    _ => {}
                }
            }
        }
        let mut points: Vec<_> = series
            .into_values()
            .filter(|point| point.lat != 0.0 || point.lon != 0.0)
            .collect();
        let limit = limit.clamp(1, 2000) as usize;
        if points.len() > limit {
            points = points.split_off(points.len() - limit);
        }
        Ok(points)
    }

    async fn query_metric(
        &self,
        metric: &str,
        vehicle_id: &str,
        start: i64,
        end: i64,
    ) -> AppResult<Vec<(i64, f64)>> {
        let query = format!("{metric}{{vehicle_id=\"{}\"}}", escape_label(vehicle_id));
        let mut request = self
            .client
            .get(format!("{}/api/v1/query_range", self.base))
            .query(&[
                ("query", query.as_str()),
                ("start", &start.to_string()),
                ("end", &end.to_string()),
                ("step", "1s"),
            ]);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|error| {
            tracing::warn!(%error, "tsink query request failed");
            AppError::internal("tsink 查询失败")
        })?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "tsink 查询失败（{}）",
                response.status()
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|_| AppError::internal("tsink 查询响应解析失败"))?;
        let Some(result) = value
            .get("data")
            .and_then(|data| data.get("result"))
            .and_then(Value::as_array)
        else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for series in result {
            let Some(values) = series.get("values").and_then(Value::as_array) else {
                continue;
            };
            for pair in values {
                let Some(items) = pair.as_array() else {
                    continue;
                };
                let seconds = items.first().and_then(Value::as_f64).unwrap_or(0.0);
                let ts_ms = (seconds * 1000.0) as i64;
                let value = items
                    .get(1)
                    .and_then(Value::as_str)
                    .and_then(|v| v.parse::<f64>().ok());
                if let Some(value) = value {
                    out.push((ts_ms, value));
                }
            }
        }
        Ok(out)
    }
}

async fn write_sample(
    client: &Client,
    base: &str,
    token: Option<&str>,
    sample: &Sample,
) -> AppResult<()> {
    let id = escape_label(&sample.vehicle_id);
    let body = format!(
        "tsp_lat{{vehicle_id=\"{id}\"}} {} {ts}\n\
         tsp_lon{{vehicle_id=\"{id}\"}} {} {ts}\n\
         tsp_speed{{vehicle_id=\"{id}\"}} {} {ts}\n\
         tsp_heading{{vehicle_id=\"{id}\"}} {} {ts}\n\
         tsp_ignition{{vehicle_id=\"{id}\"}} {} {ts}\n\
         tsp_soc{{vehicle_id=\"{id}\"}} {} {ts}\n\
         tsp_odometer{{vehicle_id=\"{id}\"}} {} {ts}\n",
        sample.lat,
        sample.lon,
        sample.speed,
        sample.heading,
        if sample.ignition { 1 } else { 0 },
        sample.soc,
        sample.odometer_km,
        ts = sample.ts_ms,
        id = id
    );
    let mut request = client
        .post(format!("{base}/api/v1/import/prometheus"))
        .header("Content-Type", "text/plain")
        .body(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|error| {
        tracing::warn!(%error, "tsink import request failed");
        AppError::internal("tsink 写入失败")
    })?;
    if !response.status().is_success() {
        return Err(AppError::internal(format!(
            "tsink 写入失败（{}）",
            response.status()
        )));
    }
    Ok(())
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
