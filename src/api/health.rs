use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::{models::HealthResponse, state::AppState, util::now_iso};

pub(crate) async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let database = state.db.ping().await.is_ok();
    let cluster = state.cluster.status();
    let body = HealthResponse {
        status: if database { "ok" } else { "degraded" },
        version: env!("CARGO_PKG_VERSION"),
        database,
        cluster_enabled: cluster.enabled,
        cluster_leader: cluster.leader,
        timeseries_backend: state.config.timeseries_backend.as_str().to_string(),
        tdengine: state.tdengine.is_some(),
        timestamp: now_iso(),
    };
    let status = if database {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body))
}

pub(crate) async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let database = state.db.ping().await.is_ok();
    let cluster = state.cluster.status();
    let ready = database;
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(serde_json::json!({
            "ready": ready,
            "database": database,
            "cluster_enabled": cluster.enabled,
            "cluster_leader": cluster.leader,
            "timeseries_backend": state.config.timeseries_backend.as_str(),
            "tdengine": state.tdengine.is_some(),
        })),
    )
}
