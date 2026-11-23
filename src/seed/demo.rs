use chrono::{Duration, Utc};

use super::builders;
use super::model::*;

use crate::{
    auth::hash_password,
    db::Db,
    error::{AppError, AppResult},
    util::{haversine_m, point_in_polygon},
};

pub async fn seed_if_empty(db: &Db) -> AppResult<bool> {
    let seed_lock = db.acquire_seed_lock().await?;
    if db.user_count().await? > 0 {
        seed_lock.release().await;
        return Ok(false);
    }

    for (username, password, display_name, role, org_name) in [
        ("admin", "admin123", "张伟", "admin", ""),
        (
            "operator",
            "operator123",
            "李敏",
            "operator",
            "星途出行 · 浦东车队",
        ),
        (
            "viewer",
            "viewer123",
            "王磊",
            "viewer",
            "星途出行 · 虹桥车队",
        ),
    ] {
        let hash = hash_password(password)?;
        db.create_user(username, &hash, display_name, role, org_name)
            .await?;
    }

    let demo: DemoData = serde_json::from_str(include_str!("../../seed/demo.json"))
        .map_err(|error| AppError::internal(format!("demo seed data is invalid: {error}")))?;
    if demo.vehicles.len() < 10 {
        return Err(AppError::internal("demo seed needs at least ten vehicles"));
    }

    let now = Utc::now();
    let created_at = iso(now - Duration::days(120));
    let mut rng = crate::util::Lcg::new(20_261_115);

    let crate::seed::fleet::FleetSeed {
        vehicles,
        states,
        terminals,
        all_points,
        active_trips,
    } = crate::seed::fleet::build(&demo, &mut rng, now, created_at);
    let trips = crate::seed::fleet::trips(&vehicles, &states, &mut rng, now);
    let fences = builders::fences(&demo, now);
    let mut geofence_states = Vec::new();
    for fence in &fences {
        for state in &states {
            let (Some(lat), Some(lon)) = (state.lat, state.lon) else {
                continue;
            };
            let inside = if fence.kind == "polygon" {
                fence
                    .polygon
                    .as_ref()
                    .map(|polygon| point_in_polygon(lat, lon, polygon))
                    .unwrap_or(false)
            } else {
                haversine_m(lat, lon, fence.center_lat, fence.center_lon) <= fence.radius_m
            };
            if inside {
                geofence_states.push((state.vehicle_id.clone(), fence.id.clone(), iso(now)));
            }
        }
    }

    let alerts = builders::alerts(&demo, &states, &vehicles, now);
    let commands = builders::commands(&demo, &vehicles, now);

    let mut tx = db.pool().begin().await?;

    let vehicle_sql = db.sql(
        "INSERT INTO vehicles
         (id, vin, plate_no, model, color, energy_type, driver_name, driver_phone,
          org_name, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    );
    for vehicle in &vehicles {
        db.query(&vehicle_sql)
            .bind(&vehicle.id)
            .bind(&vehicle.vin)
            .bind(&vehicle.plate_no)
            .bind(&vehicle.model)
            .bind(&vehicle.color)
            .bind(&vehicle.energy_type)
            .bind(&vehicle.driver_name)
            .bind(&vehicle.driver_phone)
            .bind(&vehicle.org_name)
            .bind(&vehicle.created_at)
            .execute(&mut tx)
            .await?;
    }

    let state_sql = db.sql(
        "INSERT INTO vehicle_state
         (vehicle_id, lat, lon, speed, heading, ignition, soc, odometer_km, status, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    );
    for state in &states {
        db.query(&state_sql)
            .bind(&state.vehicle_id)
            .bind(state.lat)
            .bind(state.lon)
            .bind(state.speed)
            .bind(state.heading)
            .bind(state.ignition as i64)
            .bind(state.soc)
            .bind(state.odometer_km)
            .bind(&state.status)
            .bind(&state.updated_at)
            .execute(&mut tx)
            .await?;
    }

    let terminal_sql = db.sql(
        "INSERT INTO terminals
         (id, imei, phone, iccid, model, firmware, protocol, vehicle_id, status, auth_code,
          last_heartbeat_at, created_at, bound_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'V1.4.2', 'JT/T 808-2019', ?6, ?7, ?8, ?9, ?10, ?11)",
    );
    for terminal in &terminals {
        db.query(&terminal_sql)
            .bind(&terminal.id)
            .bind(&terminal.imei)
            .bind(&terminal.phone)
            .bind(&terminal.iccid)
            .bind(&terminal.model)
            .bind(&terminal.vehicle_id)
            .bind(&terminal.status)
            .bind(&terminal.auth_code)
            .bind(terminal.last_heartbeat_at.as_deref())
            .bind(&terminal.created_at)
            .bind(&terminal.bound_at)
            .execute(&mut tx)
            .await?;
    }

    let telemetry_sql = db.sql(
        "INSERT INTO telemetry
         (vehicle_id, ts, lat, lon, speed, heading, ignition, soc, odometer_km, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    );
    for point in &all_points {
        db.query(&telemetry_sql)
            .bind(&point.vehicle_id)
            .bind(&point.ts)
            .bind(point.lat)
            .bind(point.lon)
            .bind(point.speed)
            .bind(point.heading)
            .bind(point.ignition as i64)
            .bind(point.soc)
            .bind(point.odometer_km)
            .bind(&point.source)
            .execute(&mut tx)
            .await?;
    }

    let trip_sql = db.sql(
        "INSERT INTO trips
         (id, vehicle_id, start_ts, end_ts, distance_km, avg_speed, max_speed,
          start_lat, start_lon, end_lat, end_lon, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    );
    for trip in trips.iter().chain(active_trips.iter()) {
        db.query(&trip_sql)
            .bind(&trip.id)
            .bind(&trip.vehicle_id)
            .bind(&trip.start_ts)
            .bind(trip.end_ts.as_deref())
            .bind(trip.distance_km)
            .bind(trip.avg_speed)
            .bind(trip.max_speed)
            .bind(trip.start_lat)
            .bind(trip.start_lon)
            .bind(trip.end_lat)
            .bind(trip.end_lon)
            .bind(&trip.status)
            .execute(&mut tx)
            .await?;
    }

    let fence_sql = db.sql(
        "INSERT INTO geofences
         (id, name, kind, center_lat, center_lon, radius_m, polygon, color, active, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    );
    for fence in &fences {
        let polygon = fence
            .polygon
            .as_ref()
            .map(|value| serde_json::to_string(value).unwrap_or_default());
        db.query(&fence_sql)
            .bind(&fence.id)
            .bind(&fence.name)
            .bind(&fence.kind)
            .bind(fence.center_lat)
            .bind(fence.center_lon)
            .bind(fence.radius_m)
            .bind(polygon.as_deref())
            .bind(&fence.color)
            .bind(fence.active as i64)
            .bind(&fence.created_at)
            .execute(&mut tx)
            .await?;
    }

    let geofence_state_sql = db.sql(
        "INSERT INTO geofence_states (vehicle_id, geofence_id, inside, updated_at)
         VALUES (?1, ?2, 1, ?3)",
    );
    for (vehicle_id, fence_id, updated_at) in &geofence_states {
        db.query(&geofence_state_sql)
            .bind(vehicle_id)
            .bind(fence_id)
            .bind(updated_at)
            .execute(&mut tx)
            .await?;
    }

    let alert_sql = db.sql(
        "INSERT INTO alerts
         (id, vehicle_id, alert_type, level, title, message, lat, lon, ts,
          acknowledged, ack_by, ack_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    );
    for alert in &alerts {
        db.query(&alert_sql)
            .bind(&alert.0)
            .bind(&alert.1)
            .bind(&alert.2)
            .bind(&alert.3)
            .bind(&alert.4)
            .bind(&alert.5)
            .bind(alert.6)
            .bind(alert.7)
            .bind(&alert.8)
            .bind(alert.9 as i64)
            .bind(alert.10.as_deref())
            .bind(alert.11.as_deref())
            .execute(&mut tx)
            .await?;
    }

    let command_sql = db.sql(
        "INSERT INTO commands
         (id, vehicle_id, command, status, requested_by, payload, result,
          created_at, sent_at, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    );
    for command in &commands {
        db.query(&command_sql)
            .bind(&command.id)
            .bind(&command.vehicle_id)
            .bind(&command.command)
            .bind(&command.status)
            .bind(&command.requested_by)
            .bind(command.payload.as_deref())
            .bind(command.result.as_deref())
            .bind(&command.created_at)
            .bind(command.sent_at.as_deref())
            .bind(command.completed_at.as_deref())
            .execute(&mut tx)
            .await?;
    }

    let audit_sql = db.sql(
        "INSERT INTO audit_logs (user_id, username, action, target, detail, created_at)
         VALUES (?1, (SELECT username FROM users WHERE id = ?1), ?2, ?3, ?4, ?5)",
    );
    for (user_id, action, target, detail) in [
        (
            Some("admin"),
            "vehicle.create",
            "veh_001",
            "Create demo vehicle and bind terminal",
        ),
        (
            Some("operator"),
            "geofence.update",
            "geo_zhangjiang",
            "Set fence radius to 2600m",
        ),
        (
            Some("admin"),
            "command.dispatch",
            "veh_002",
            "Remote climate control",
        ),
    ] {
        db.query(&audit_sql)
            .bind(user_id)
            .bind(action)
            .bind(target)
            .bind(detail)
            .bind(iso(now))
            .execute(&mut tx)
            .await?;
    }

    tx.commit().await?;
    seed_lock.release().await;

    Ok(true)
}
