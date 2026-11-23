use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{AckRequest, Alert, AlertListQuery, Page},
    security::access::{visible_value, Role},
    security::pii::redact_text,
    state::AppState,
};

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<AlertListQuery>,
) -> AppResult<Json<Page<Alert>>> {
    let mut page = state.db.list_alerts(query, user.org_filter()).await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    for item in &mut page.items {
        mask_alert(item, role);
    }
    Ok(Json(page))
}

pub async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<AlertListQuery>,
) -> AppResult<Response> {
    let mut page = state
        .db
        .list_alerts(
            AlertListQuery {
                status: query.status,
                level: query.level,
                vehicle_id: query.vehicle_id,
                page: Some(1),
                page_size: Some(1000),
            },
            user.org_filter(),
        )
        .await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    let mut csv =
        String::from("id,vehicle_id,plate_no,alert_type,level,title,message,ts,acknowledged\n");
    for item in &mut page.items {
        mask_alert(item, role);
        csv.push_str(&format!(
            "{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{}\n",
            item.id,
            csv_escape(&item.vehicle_id),
            csv_escape(item.plate_no.as_deref().unwrap_or("")),
            csv_escape(&item.alert_type),
            csv_escape(&item.level),
            csv_escape(&item.title),
            csv_escape(&item.message),
            csv_escape(&item.ts),
            item.acknowledged,
        ));
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "alert.export",
            "alerts",
            &format!("exported {} rows", page.items.len()),
        )
        .await?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
        csv,
    )
        .into_response())
}

fn csv_escape(value: &str) -> String {
    value.replace('"', "\"\"")
}

pub async fn ack(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<AckRequest>,
) -> AppResult<Json<Alert>> {
    user.require_write()?;
    let existing = state
        .db
        .get_alert(&id)
        .await?
        .ok_or_else(|| AppError::not_found("告警不存在"))?;
    let vehicle = state
        .db
        .get_vehicle(&existing.vehicle_id)
        .await?
        .ok_or_else(|| AppError::not_found("告警不存在"))?;
    if !user.can_access_org(&vehicle.vehicle.org_name) {
        return Err(AppError::not_found("告警不存在"));
    }
    let note = payload.note.unwrap_or_default();
    let alert = state
        .db
        .acknowledge_alert(&id, &user.username)
        .await?
        .ok_or_else(|| AppError::not_found("告警不存在"))?;
    let detail = if note.is_empty() {
        "确认告警".to_string()
    } else {
        format!("确认告警：{note}")
    };
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "alert.acknowledge",
            &id,
            &detail,
        )
        .await?;
    Ok(Json(alert))
}

fn mask_alert(item: &mut Alert, role: Role) {
    if let Some(plate) = &item.plate_no {
        item.plate_no = Some(visible_value(role, "plate", plate));
    }
    item.message = redact_text(&item.message);
    if let Some(ack_by) = &item.ack_by {
        item.ack_by = Some(visible_value(role, "driver_name", ack_by));
    }
}

pub async fn ack_all(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    user.require_admin()?;
    let count = state.db.acknowledge_all_alerts(&user.username).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "alert.acknowledge_all",
            "alerts",
            &format!("批量确认 {count} 条告警"),
        )
        .await?;
    Ok(Json(serde_json::json!({ "acknowledged": count })))
}
