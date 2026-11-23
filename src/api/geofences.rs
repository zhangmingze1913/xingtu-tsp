use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{CreateGeofenceRequest, Geofence, UpdateGeofenceRequest},
    state::AppState,
};

pub async fn list(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<Geofence>>> {
    Ok(Json(state.db.list_geofences(user.org_filter()).await?))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Geofence>> {
    state
        .db
        .get_geofence(&id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("电子围栏不存在"))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<CreateGeofenceRequest>,
) -> AppResult<(StatusCode, Json<Geofence>)> {
    user.require_write()?;
    let org = if user.is_admin() {
        String::new()
    } else {
        user.org_name.clone()
    };
    let fence = state.db.create_geofence(payload, &org).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "geofence.create",
            &fence.id,
            &format!("创建电子围栏「{}」", fence.name),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(fence)))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<UpdateGeofenceRequest>,
) -> AppResult<Json<Geofence>> {
    user.require_write()?;
    let existing = state
        .db
        .get_geofence(&id)
        .await?
        .ok_or_else(|| AppError::not_found("电子围栏不存在"))?;
    if !existing.org_name.is_empty() && !user.can_access_org(&existing.org_name) {
        return Err(AppError::not_found("电子围栏不存在"));
    }
    let fence = state
        .db
        .update_geofence(&id, payload)
        .await?
        .ok_or_else(|| AppError::not_found("电子围栏不存在"))?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "geofence.update",
            &id,
            &format!("更新电子围栏「{}」", fence.name),
        )
        .await?;
    Ok(Json(fence))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    user.require_admin()?;
    if !state.db.delete_geofence(&id).await? {
        return Err(AppError::not_found("电子围栏不存在"));
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "geofence.delete",
            &id,
            "删除电子围栏",
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
