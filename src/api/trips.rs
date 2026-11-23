use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{Page, TelemetryPoint, Trip, TripListQuery},
    security::access::{visible_value, Role},
    state::AppState,
};

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<TripListQuery>,
) -> AppResult<Json<Page<Trip>>> {
    let mut page = state
        .db
        .list_trips(
            query.vehicle_id,
            query.page,
            query.page_size,
            user.org_filter(),
        )
        .await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    for item in &mut page.items {
        mask_trip(item, role);
    }
    Ok(Json(page))
}

fn mask_trip(item: &mut Trip, role: Role) {
    if let Some(plate) = &item.plate_no {
        item.plate_no = Some(visible_value(role, "plate", plate));
    }
}

pub async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<TripListQuery>,
) -> AppResult<Response> {
    let mut page = state
        .db
        .list_trips(query.vehicle_id, Some(1), Some(1000), user.org_filter())
        .await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    let mut csv = String::from(
        "id,vehicle_id,plate_no,start_ts,end_ts,distance_km,avg_speed,max_speed,status\n",
    );
    for item in &mut page.items {
        mask_trip(item, role);
        csv.push_str(&format!(
            "{},\"{}\",\"{}\",\"{}\",\"{}\",{},{},{},\"{}\"\n",
            item.id,
            csv_escape(&item.vehicle_id),
            csv_escape(item.plate_no.as_deref().unwrap_or("")),
            csv_escape(&item.start_ts),
            csv_escape(item.end_ts.as_deref().unwrap_or("")),
            item.distance_km,
            item.avg_speed,
            item.max_speed,
            csv_escape(&item.status),
        ));
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "trip.export",
            "trips",
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

pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let trip = state
        .db
        .get_trip(&id)
        .await?
        .ok_or_else(|| AppError::not_found("行程不存在"))?;
    let vehicle = state
        .db
        .get_vehicle(&trip.vehicle_id)
        .await?
        .ok_or_else(|| AppError::not_found("行程不存在"))?;
    if !user.can_access_org(&vehicle.vehicle.org_name) {
        return Err(AppError::not_found("行程不存在"));
    }
    let points: Vec<TelemetryPoint> = state.db.trip_points(&id).await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    let mut trip = trip;
    mask_trip(&mut trip, role);
    Ok(Json(serde_json::json!({
        "trip": trip,
        "points": points
    })))
}
