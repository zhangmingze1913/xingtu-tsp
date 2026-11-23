use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{Command, CommandListQuery, CreateCommandRequest, Page},
    services,
    state::AppState,
};

use super::vehicles::require_vehicle;

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<CommandListQuery>,
) -> AppResult<Json<Page<Command>>> {
    Ok(Json(
        state.db.list_commands(query, user.org_filter()).await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Command>> {
    let command = state
        .db
        .get_command(&id)
        .await?
        .ok_or_else(|| AppError::not_found("指令不存在"))?;
    let vehicle = state
        .db
        .get_vehicle(&command.vehicle_id)
        .await?
        .ok_or_else(|| AppError::not_found("指令不存在"))?;
    if !user.can_access_org(&vehicle.vehicle.org_name) {
        return Err(AppError::not_found("指令不存在"));
    }
    Ok(Json(command))
}

pub async fn list_commands(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<CommandListQuery>,
) -> AppResult<Json<Page<Command>>> {
    require_vehicle(&state, &user, &id).await?;
    Ok(Json(
        state
            .db
            .list_commands(
                CommandListQuery {
                    status: query.status,
                    vehicle_id: Some(id),
                    page: query.page,
                    page_size: query.page_size,
                },
                user.org_filter(),
            )
            .await?,
    ))
}

pub async fn create_command(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<CreateCommandRequest>,
) -> AppResult<(StatusCode, Json<Command>)> {
    user.require_write()?;
    if payload.command == "raw_808" {
        user.require_admin()?;
    }
    require_vehicle(&state, &user, &id).await?;
    if state.config.command_mode == "disabled" {
        return Err(AppError::conflict(
            "远程指令通道已禁用；演示环境请设置 TSP_COMMAND_MODE=simulated",
        ));
    }
    let command = state
        .db
        .create_command(&id, &payload.command, payload.payload, &user.username)
        .await?;
    use std::sync::atomic::Ordering;
    state
        .metrics
        .commands_created
        .fetch_add(1, Ordering::Relaxed);
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "command.dispatch",
            &command.id,
            &format!(
                "向车辆 {} 下发指令 {}",
                command.plate_no.as_deref().unwrap_or("-"),
                command.command
            ),
        )
        .await?;
    services::commands::dispatch(state.clone(), command.clone());
    Ok((StatusCode::ACCEPTED, Json(command)))
}
