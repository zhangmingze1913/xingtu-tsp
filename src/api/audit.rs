use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::{
    auth::AuthUser,
    error::AppResult,
    models::{AuditLog, AuditQuery, Page},
    security::pii::{mask_name, redact_text},
    state::AppState,
};

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<AuditQuery>,
) -> AppResult<Json<Page<AuditLog>>> {
    user.require_admin()?;
    let mut page = state.db.list_audit(query).await?;
    for item in &mut page.items {
        if let Some(username) = &item.username {
            item.username = Some(mask_name(username));
        }
        item.detail = redact_text(&item.detail);
    }
    Ok(Json(page))
}

pub async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<AuditQuery>,
) -> AppResult<Response> {
    user.require_admin()?;
    let page = state
        .db
        .list_audit(AuditQuery {
            action: query.action,
            page: Some(1),
            page_size: Some(200),
        })
        .await?;
    let mut csv = String::from("id,user_id,username,action,target,detail,created_at\n");
    for item in page.items {
        csv.push_str(&format!(
            "{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            item.id,
            escape(&item.user_id.unwrap_or_default()),
            escape(&mask_name(&item.username.unwrap_or_default())),
            escape(&item.action),
            escape(&item.target),
            escape(&redact_text(&item.detail)),
            escape(&item.created_at),
        ));
    }
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
        csv,
    )
        .into_response())
}

fn escape(value: &str) -> String {
    value.replace('"', "\"\"")
}
