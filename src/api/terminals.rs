use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{BindTerminalRequest, Terminal, TerminalRow},
    security::access::{visible_value, Role},
    state::AppState,
};

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<Vec<TerminalRow>>> {
    let mut rows = state.db.list_terminals().await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    for row in &mut rows {
        mask_terminal(&mut row.terminal, role);
    }
    Ok(Json(rows))
}

pub async fn export(State(state): State<AppState>, user: AuthUser) -> AppResult<Response> {
    let mut rows = state.db.list_terminals().await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    let mut csv = String::from("id,imei,phone,iccid,model,vehicle_id,status\n");
    for row in &mut rows {
        mask_terminal(&mut row.terminal, role);
        csv.push_str(&format!(
            "{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            row.terminal.id,
            csv_escape(&row.terminal.imei),
            csv_escape(&row.terminal.phone),
            csv_escape(&row.terminal.iccid),
            csv_escape(&row.terminal.model),
            csv_escape(row.terminal.vehicle_id.as_deref().unwrap_or("")),
            csv_escape(&row.terminal.status),
        ));
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.export",
            "terminals",
            &format!("exported {} rows", rows.len()),
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

fn mask_terminal(terminal: &mut Terminal, role: Role) {
    terminal.phone = visible_value(role, "phone", &terminal.phone);
    terminal.iccid = visible_value(role, "iccid", &terminal.iccid);
    terminal.imei = visible_value(role, "imei", &terminal.imei);
}

pub async fn bind(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<BindTerminalRequest>,
) -> AppResult<Json<Terminal>> {
    user.require_write()?;
    let terminal = state.db.bind_terminal(&id, payload.vehicle_id).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.update_binding",
            &id,
            "更新终端与车辆的绑定关系",
        )
        .await?;
    Ok(Json(terminal))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Terminal>> {
    state
        .db
        .get_terminal(&id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("终端不存在"))
}
