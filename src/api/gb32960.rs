use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    standards::gb32960_ingest::{ingest, Gb32960IngestResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct Gb32960IngestRequest {
    pub frame_hex: String,
}

/// Administrative ingestion endpoint for tests and platform bridges.
/// Real terminals should be attached through a gateway with device auth.
pub async fn ingest_frame(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<Gb32960IngestRequest>,
) -> AppResult<Json<Gb32960IngestResult>> {
    user.require_write()?;
    let raw = decode_hex(&payload.frame_hex)?;
    Ok(Json(ingest(&state.db, &raw).await?))
}

fn decode_hex(value: &str) -> AppResult<Vec<u8>> {
    let compact: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    if !compact.len().is_multiple_of(2) {
        return Err(AppError::bad_request("frame_hex 长度必须为偶数"));
    }
    let mut out = Vec::with_capacity(compact.len() / 2);
    for index in (0..compact.len()).step_by(2) {
        let byte = u8::from_str_radix(&compact[index..index + 2], 16)
            .map_err(|_| AppError::bad_request("frame_hex 含非法十六进制"))?;
        out.push(byte);
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct Gb32960DeviceView {
    pub vin: String,
    pub enabled: bool,
    pub note: String,
}

#[derive(Debug, Deserialize)]
pub struct Gb32960DeviceUpdate {
    pub enabled: bool,
    #[serde(default)]
    pub note: String,
}

pub async fn list_devices(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<Vec<Gb32960DeviceView>>> {
    user.require_admin()?;
    let items = state
        .db
        .list_gb32960_devices()
        .await?
        .into_iter()
        .map(|(vin, enabled, note)| Gb32960DeviceView { vin, enabled, note })
        .collect();
    Ok(Json(items))
}

pub async fn set_device(
    State(state): State<AppState>,
    user: AuthUser,
    Path(vin): Path<String>,
    Json(payload): Json<Gb32960DeviceUpdate>,
) -> AppResult<Json<Gb32960DeviceView>> {
    user.require_admin()?;
    let vin = vin.trim().to_ascii_uppercase();
    if vin.len() != 17 {
        return Err(AppError::bad_request("VIN 必须为 17 位"));
    }
    state
        .db
        .set_gb32960_device(&vin, payload.enabled, &payload.note)
        .await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "gb32960.device_update",
            &vin,
            &format!(
                "设备{}；备注：{}",
                if payload.enabled { "启用" } else { "禁用" },
                payload.note
            ),
        )
        .await?;
    Ok(Json(Gb32960DeviceView {
        vin,
        enabled: payload.enabled,
        note: payload.note,
    }))
}
