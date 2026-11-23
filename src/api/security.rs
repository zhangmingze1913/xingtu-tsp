use axum::Json;

use crate::{auth::AuthUser, error::AppResult, security::compliance::snapshot};

pub async fn compliance(user: AuthUser) -> AppResult<Json<serde_json::Value>> {
    user.require_admin()?;
    Ok(Json(serde_json::json!({
        "scope": "code-side evidence; not a certification",
        "controls": snapshot(),
    })))
}
