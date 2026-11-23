use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{IngestOutcome, IngestRequest, LocationSample, TelemetryPoint, TrajectoryQuery},
    services,
    state::AppState,
    util::validate_coord,
};

use super::vehicles::require_vehicle;

pub async fn trajectory(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<TrajectoryQuery>,
) -> AppResult<Json<Vec<TelemetryPoint>>> {
    require_vehicle(&state, &user, &id).await?;
    if let Some(from) = &query.from {
        chrono::DateTime::parse_from_rfc3339(from)
            .map_err(|_| AppError::bad_request("from 必须是 RFC3339 时间"))?;
    }
    if let Some(to) = &query.to {
        chrono::DateTime::parse_from_rfc3339(to)
            .map_err(|_| AppError::bad_request("to 必须是 RFC3339 时间"))?;
    }
    let limit = query.limit.unwrap_or(1000);
    let from = query
        .from
        .unwrap_or_else(|| crate::util::iso_from_now_minutes(-120));
    let to = query.to.unwrap_or_else(crate::util::now_iso);

    if let Some(tsink) = &state.tsink {
        match tsink.query(&id, &from, &to, limit).await {
            Ok(points) if !points.is_empty() => return Ok(Json(points)),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, vehicle = %id, "tsink trajectory fallback"),
        }
    }
    if let Some(tdengine) = &state.tdengine {
        match tdengine.query(&id, &from, &to, limit).await {
            Ok(points) if !points.is_empty() => return Ok(Json(points)),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, vehicle = %id, "TDengine trajectory fallback"),
        }
    }

    Ok(Json(
        state
            .db
            .trajectory(&id, Some(from), Some(to), limit)
            .await?,
    ))
}

pub async fn trajectory_export(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<TrajectoryQuery>,
) -> AppResult<Response> {
    require_vehicle(&state, &user, &id).await?;
    let limit = query.limit.unwrap_or(1000).min(5000);
    let from = query
        .from
        .unwrap_or_else(|| crate::util::iso_from_now_minutes(-120));
    let to = query.to.unwrap_or_else(crate::util::now_iso);
    let points = state
        .db
        .trajectory(&id, Some(from), Some(to), limit)
        .await?;
    let mut csv = String::from("ts,lat,lon,speed,heading,ignition,soc,odometer_km,source\n");
    for point in points {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},\"{}\"\n",
            escape_csv(&point.ts),
            point.lat,
            point.lon,
            point.speed,
            point.heading,
            point.ignition,
            point.soc,
            point.odometer_km,
            escape_csv(&point.source),
        ));
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "trajectory.export",
            &id,
            &format!("exported {} points", csv.lines().count().saturating_sub(1)),
        )
        .await?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
        csv,
    )
        .into_response())
}

fn escape_csv(value: &str) -> String {
    value.replace('"', "\"\"")
}

pub async fn ingest(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<IngestRequest>,
) -> AppResult<(StatusCode, Json<IngestOutcome>)> {
    user.require_write()?;
    require_vehicle(&state, &user, &payload.vehicle_id).await?;
    validate_coord(payload.lat, payload.lon).map_err(AppError::bad_request)?;
    if payload.speed < 0.0 || payload.speed > 300.0 {
        return Err(AppError::bad_request("车速必须在 0 至 300 km/h 之间"));
    }
    if let Some(soc) = payload.soc {
        if !(0.0..=100.0).contains(&soc) {
            return Err(AppError::bad_request("SOC 必须在 0 至 100 之间"));
        }
    }
    let ts = payload.ts.unwrap_or_else(crate::util::now_iso);
    chrono::DateTime::parse_from_rfc3339(&ts)
        .map_err(|_| AppError::bad_request("ts 必须是 RFC3339 时间"))?;
    let sample = LocationSample {
        vehicle_id: payload.vehicle_id,
        ts,
        lat: payload.lat,
        lon: payload.lon,
        speed: payload.speed,
        heading: payload.heading.rem_euclid(360.0),
        ignition: payload.ignition,
        soc: payload.soc,
        odometer_km: payload.odometer_km,
        source: payload.source,
        extra_json: None,
    };
    let outcome = services::telemetry::ingest(&state, sample)
        .await?
        .ok_or_else(|| AppError::not_found("车辆不存在"))?;
    Ok((StatusCode::CREATED, Json(outcome)))
}
