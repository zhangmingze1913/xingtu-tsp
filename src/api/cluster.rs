use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::{auth::AuthUser, state::AppState};

pub async fn status(State(state): State<AppState>, _user: AuthUser) -> Json<Value> {
    let mut body = serde_json::to_value(state.cluster.status()).unwrap_or_else(|_| json!({}));
    let event_bus = if state.config.iggy_url.is_some() {
        "iggy"
    } else if state.cluster.enabled() {
        "redis"
    } else {
        "local"
    };
    body["event_bus"] = json!(event_bus);
    Json(body)
}

pub async fn metrics(State(state): State<AppState>) -> Response {
    let body = state.metrics.render(&state.cluster.status());
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
