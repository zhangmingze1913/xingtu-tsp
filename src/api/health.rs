use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::{models::HealthResponse, state::AppState, util::now_iso};

pub(crate) async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let database = state.db.ping().await.is_ok();
    let cluster = state.cluster.status();
    let tdengine_ready = if let Some(tdengine) = &state.tdengine {
        Some(tdengine.ping().await.is_ok())
    } else {
        None
    };
    let body = HealthResponse {
        status: if database { "ok" } else { "degraded" },
        version: env!("CARGO_PKG_VERSION"),
        database,
        database_engine: state.db.engine_name().to_string(),
        cluster_enabled: cluster.enabled,
        cluster_leader: cluster.leader,
        timeseries_backend: state.config.timeseries_backend.as_str().to_string(),
        tdengine: state.tdengine.is_some(),
        tdengine_ready,
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
    let tdengine_ready = if let Some(tdengine) = &state.tdengine {
        Some(tdengine.ping().await.is_ok())
    } else {
        None
    };
    let tdengine_required =
        state.config.timeseries_backend == crate::config::TimeSeriesBackend::Tdengine;
    let ready = database && (!tdengine_required || tdengine_ready == Some(true));
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
            "database_engine": state.db.engine_name(),
            "timeseries_backend": state.config.timeseries_backend.as_str(),
            "tdengine": state.tdengine.is_some(),
            "tdengine_ready": tdengine_ready,
        })),
    )
}
