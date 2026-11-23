use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{
        BindVehicleTerminalRequest, CreateVehicleRequest, MapQuery, MapVehicle, Page, Trip,
        TripListQuery, UpdateVehicleRequest, VehicleDetail,
    },
    security::access::{can_view_key, visible_value, Role},
    state::AppState,
};

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<crate::models::VehicleListQuery>,
) -> AppResult<Json<Page<VehicleDetail>>> {
    let mut page = state
        .db
        .list_vehicles(
            query.search,
            query.status,
            query.page,
            query.page_size,
            user.org_filter(),
        )
        .await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    if can_view_key(role, "phone") || can_view_key(role, "driver_name") {
        let mut vehicles: Vec<&mut crate::models::Vehicle> = page
            .items
            .iter_mut()
            .map(|item| &mut item.vehicle)
            .collect();
        state.db.hydrate_vehicles_pii(&mut vehicles).await?;
    }
    for item in &mut page.items {
        mask_vehicle(item, role);
    }
    Ok(Json(page))
}

pub async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<crate::models::VehicleListQuery>,
) -> AppResult<Response> {
    let mut page = state
        .db
        .list_vehicles(
            query.search,
            query.status,
            Some(1),
            Some(1000),
            user.org_filter(),
        )
        .await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    if can_view_key(role, "phone") || can_view_key(role, "driver_name") {
        let mut vehicles: Vec<&mut crate::models::Vehicle> = page
            .items
            .iter_mut()
            .map(|item| &mut item.vehicle)
            .collect();
        state.db.hydrate_vehicles_pii(&mut vehicles).await?;
    }
    let mut csv = String::from("id,vin,plate_no,model,driver_name,driver_phone,org_name,status\n");
    for item in &mut page.items {
        mask_vehicle(item, role);
        csv.push_str(&format!(
            "{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            item.vehicle.id,
            csv_escape(&item.vehicle.vin),
            csv_escape(&item.vehicle.plate_no),
            csv_escape(&item.vehicle.model),
            csv_escape(&item.vehicle.driver_name),
            csv_escape(&item.vehicle.driver_phone),
            csv_escape(&item.vehicle.org_name),
            csv_escape(&item.state.status),
        ));
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "vehicle.export",
            "vehicles",
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

pub async fn map(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<MapQuery>,
) -> AppResult<Json<Vec<MapVehicle>>> {
    if let Some(bbox) = query.bbox {
        let (min_lon, min_lat, max_lon, max_lat) = parse_bbox(&bbox)?;
        return Ok(Json(
            state
                .db
                .vehicles_in_bbox(
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    query.status,
                    user.org_filter(),
                )
                .await?,
        ));
    }
    Ok(Json(
        state
            .db
            .map_vehicles(query.status, user.org_filter())
            .await?,
    ))
}

pub async fn nearby(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<crate::models::NearbyQuery>,
) -> AppResult<Json<Vec<MapVehicle>>> {
    if !(-90.0..=90.0).contains(&query.lat) || !(-180.0..=180.0).contains(&query.lon) {
        return Err(AppError::bad_request("坐标范围不正确"));
    }
    let radius_m = query.radius_m.unwrap_or(1000.0);
    if !(10.0..=50_000.0).contains(&radius_m) {
        return Err(AppError::bad_request("radius_m 必须在 10 至 50000 之间"));
    }
    Ok(Json(
        state
            .db
            .vehicles_nearby(
                query.lat,
                query.lon,
                radius_m,
                query.status,
                user.org_filter(),
            )
            .await?,
    ))
}

fn mask_vehicle(item: &mut VehicleDetail, role: Role) {
    item.vehicle.vin = visible_value(role, "vin", &item.vehicle.vin);
    item.vehicle.plate_no = visible_value(role, "plate", &item.vehicle.plate_no);
    item.vehicle.driver_name = visible_value(role, "driver_name", &item.vehicle.driver_name);
    item.vehicle.driver_phone = visible_value(role, "phone", &item.vehicle.driver_phone);
    if let Some(terminal) = &mut item.terminal {
        terminal.phone = visible_value(role, "phone", &terminal.phone);
        terminal.iccid = visible_value(role, "id_card", &terminal.iccid);
    }
}

fn parse_bbox(raw: &str) -> AppResult<(f64, f64, f64, f64)> {
    let values: Vec<f64> = raw
        .split(',')
        .map(|value| value.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| AppError::bad_request("bbox 必须是 min_lon,min_lat,max_lon,max_lat"))?;
    if values.len() != 4 {
        return Err(AppError::bad_request(
            "bbox 必须是 min_lon,min_lat,max_lon,max_lat",
        ));
    }
    let (min_lon, min_lat, max_lon, max_lat) = (values[0], values[1], values[2], values[3]);
    if min_lon >= max_lon || min_lat >= max_lat {
        return Err(AppError::bad_request("bbox 最小值必须小于最大值"));
    }
    Ok((min_lon, min_lat, max_lon, max_lat))
}

pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<VehicleDetail>> {
    let mut vehicle = state
        .db
        .get_vehicle(&id)
        .await?
        .ok_or_else(|| AppError::not_found("车辆不存在"))?;
    if !user.can_access_org(&vehicle.vehicle.org_name) {
        return Err(AppError::not_found("车辆不存在"));
    }
    state.db.hydrate_vehicle_pii(&mut vehicle.vehicle).await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    mask_vehicle(&mut vehicle, role);
    Ok(Json(vehicle))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<CreateVehicleRequest>,
) -> AppResult<(StatusCode, Json<VehicleDetail>)> {
    user.require_write()?;
    let mut payload = payload;
    if !user.is_admin() {
        payload.org_name = user.org_name.clone();
    }
    let vehicle = state.db.create_vehicle(payload).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "vehicle.create",
            &vehicle.vehicle.id,
            &format!(
                "创建车辆 {} / {}",
                vehicle.vehicle.plate_no, vehicle.vehicle.vin
            ),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(vehicle)))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<UpdateVehicleRequest>,
) -> AppResult<Json<VehicleDetail>> {
    user.require_write()?;
    require_vehicle(&state, &user, &id).await?;
    let mut vehicle = state
        .db
        .update_vehicle_and_get(&id, payload)
        .await?
        .ok_or_else(|| AppError::not_found("车辆不存在"))?;
    state.db.hydrate_vehicle_pii(&mut vehicle.vehicle).await?;
    let role = Role::parse(&user.role).unwrap_or(Role::Viewer);
    mask_vehicle(&mut vehicle, role);
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "vehicle.update",
            &id,
            "更新车辆档案",
        )
        .await?;
    Ok(Json(vehicle))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    user.require_admin()?;
    if !state.db.delete_vehicle(&id).await? {
        return Err(AppError::not_found("车辆不存在"));
    }
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "vehicle.delete",
            &id,
            "删除车辆及其关联数据",
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn trips(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<TripListQuery>,
) -> AppResult<Json<Page<Trip>>> {
    require_vehicle(&state, &user, &id).await?;
    Ok(Json(
        state
            .db
            .list_trips(Some(id), query.page, query.page_size, user.org_filter())
            .await?,
    ))
}

pub(crate) async fn require_vehicle(
    state: &AppState,
    user: &AuthUser,
    id: &str,
) -> AppResult<VehicleDetail> {
    let vehicle = state
        .db
        .get_vehicle(id)
        .await?
        .ok_or_else(|| AppError::not_found("车辆不存在"))?;
    if !user.can_access_org(&vehicle.vehicle.org_name) {
        return Err(AppError::not_found("车辆不存在"));
    }
    Ok(vehicle)
}

pub async fn bind_terminal(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<BindVehicleTerminalRequest>,
) -> AppResult<(StatusCode, Json<crate::models::Terminal>)> {
    user.require_write()?;
    require_vehicle(&state, &user, &id).await?;
    if payload.imei.trim().len() < 8 {
        return Err(AppError::bad_request("IMEI 格式不正确"));
    }
    let terminal = state
        .db
        .create_and_bind_terminal(
            &id,
            payload.imei.trim(),
            payload.phone.trim(),
            payload.iccid.trim(),
            payload.model.trim(),
        )
        .await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "terminal.bind",
            &terminal.id,
            &format!("为车辆 {} 绑定终端 {}", id, terminal.imei),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(terminal)))
}
