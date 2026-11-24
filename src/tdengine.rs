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

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    error::{AppError, AppResult},
    models::{IngestOutcome, TelemetryPoint},
};

#[derive(Clone)]
pub struct TdengineStore {
    sender: mpsc::UnboundedSender<Sample>,
    client: Client,
    base: String,
    user: String,
    password: String,
    database: String,
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

        let init = Self {
            sender: mpsc::unbounded_channel().0,
            client: client.clone(),
            base: base.clone(),
            user: user.clone(),
            password: password.clone(),
            database: database.clone(),
        };
        init.exec(&format!("CREATE DATABASE IF NOT EXISTS `{database}`"))
            .await?;
        init.exec(&format!(
            "CREATE STABLE IF NOT EXISTS `{database}`.`telemetry` (\
             ts TIMESTAMP, lat DOUBLE, lon DOUBLE, speed DOUBLE, heading DOUBLE, \
             ignition BOOL, soc DOUBLE, odometer DOUBLE\
             ) TAGS (vehicle_id NCHAR(64))"
        ))
        .await?;
        tracing::info!(url = %base, database = %database, "TDengine trajectory store enabled");

        let (sender, mut receiver) = mpsc::unbounded_channel::<Sample>();
        let writer = Self {
            sender: mpsc::unbounded_channel().0,
            client,
            base,
            user,
            password,
            database,
        };
        tokio::spawn(async move {
            while let Some(sample) = receiver.recv().await {
                if let Err(error) = writer.write_sample(&sample).await {
                    tracing::warn!(%error, vehicle = %sample.vehicle_id, "TDengine write failed");
                }
            }
        });
        Ok(Self { sender, ..init })
    }

    pub fn enqueue(&self, outcome: &IngestOutcome) {
        let Ok(ts) = DateTime::parse_from_rfc3339(&outcome.ts) else {
            return;
        };
        let _ = self.sender.send(Sample {
            vehicle_id: outcome.vehicle_id.clone(),
            ts_ms: ts.timestamp_millis(),
            lat: outcome.lat,
            lon: outcome.lon,
            speed: outcome.speed,
            heading: outcome.heading,
            ignition: outcome.ignition,
            soc: outcome.soc,
            odometer_km: outcome.odometer_km,
        });
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
}
