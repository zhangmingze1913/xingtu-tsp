use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    state::AppState,
};

pub async fn overview(State(state): State<AppState>, user: AuthUser) -> AppResult<Response> {
    let org = user.org_filter();
    if let Some(body) = state.dashboard_cache.get(org) {
        return Ok(json_response(body));
    }

    // Coalesce a cold-cache burst for the same organization. The second get
    // re-checks after waiting so followers reuse the leader's result.
    let refresh = state.dashboard_cache.lock_for(org);
    let _guard = refresh.lock().await;
    if let Some(body) = state.dashboard_cache.get(org) {
        return Ok(json_response(body));
    }

    let overview = state.db.dashboard_overview(org).await?;
    let body = serde_json::to_string(&overview)
        .map_err(|_| AppError::internal("dashboard serialization failed"))?;
    state.dashboard_cache.put(org, body.clone());
    Ok(json_response(body))
}

fn json_response(body: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}
