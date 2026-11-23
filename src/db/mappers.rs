//! Row decoding helpers and column projections shared by the repositories.

use sqlx::{any::AnyRow, Row, ValueRef};

use crate::models::{
    Alert, Command, Geofence, TelemetryPoint, Terminal, Trip, Vehicle, VehicleState,
};

// ---------------------------------------------------------------------------
// Row decoding helpers
// ---------------------------------------------------------------------------

/// Decode an integer column regardless of its PostgreSQL width / SQLite type.
pub(crate) fn get_i64(row: &AnyRow, index: usize) -> sqlx::Result<i64> {
    row.try_get::<i64, _>(index)
}

/// Decode a floating point column, tolerating SQLite integer affinity.
pub(crate) fn get_f64(row: &AnyRow, index: usize) -> sqlx::Result<f64> {
    match row.try_get::<f64, _>(index) {
        Ok(value) => Ok(value),
        Err(primary) => match row.try_get::<i64, _>(index) {
            Ok(value) => Ok(value as f64),
            Err(_) => Err(primary),
        },
    }
}

/// Decode a `0` / `1` INTEGER flag as a boolean.
pub(crate) fn get_bool(row: &AnyRow, index: usize) -> sqlx::Result<bool> {
    match row.try_get::<i64, _>(index) {
        Ok(value) => Ok(value != 0),
        Err(primary) => row.try_get::<bool, _>(index).map_err(|_| primary),
    }
}

/// Decode a nullable floating point column.
pub(crate) fn get_opt_f64(row: &AnyRow, index: usize) -> sqlx::Result<Option<f64>> {
    if row.try_get_raw(index)?.is_null() {
        Ok(None)
    } else {
        Ok(Some(get_f64(row, index)?))
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

pub(crate) fn map_vehicle(row: &AnyRow) -> sqlx::Result<Vehicle> {
    Ok(Vehicle {
        id: row.try_get(0)?,
        vin: row.try_get(1)?,
        plate_no: row.try_get(2)?,
        model: row.try_get(3)?,
        color: row.try_get(4)?,
        energy_type: row.try_get(5)?,
        driver_name: row.try_get(6)?,
        driver_phone: row.try_get(7)?,
        org_name: row.try_get(8)?,
        created_at: row.try_get(9)?,
    })
}

pub(crate) const VEHICLE_COLS: &str = "v.id, v.vin, v.plate_no, v.model, v.color, v.energy_type, \
    v.driver_name, v.driver_phone, v.org_name, v.created_at";

pub(crate) fn map_state(row: &AnyRow, offset: usize) -> sqlx::Result<VehicleState> {
    Ok(VehicleState {
        vehicle_id: row.try_get(offset)?,
        lat: get_opt_f64(row, offset + 1)?,
        lon: get_opt_f64(row, offset + 2)?,
        speed: get_f64(row, offset + 3)?,
        heading: get_f64(row, offset + 4)?,
        ignition: get_bool(row, offset + 5)?,
        soc: get_f64(row, offset + 6)?,
        odometer_km: get_f64(row, offset + 7)?,
        status: row.try_get(offset + 8)?,
        updated_at: row.try_get(offset + 9)?,
    })
}

pub(crate) const STATE_COLS: &str = "s.vehicle_id, s.lat, s.lon, s.speed, s.heading, s.ignition, \
    s.soc, s.odometer_km, s.status, s.updated_at";

pub(crate) fn map_terminal(row: &AnyRow) -> sqlx::Result<Terminal> {
    map_terminal_at(row, 0)
}

pub(crate) fn map_terminal_at(row: &AnyRow, offset: usize) -> sqlx::Result<Terminal> {
    Ok(Terminal {
        id: row.try_get(offset)?,
        imei: row.try_get(offset + 1)?,
        phone: row.try_get(offset + 2)?,
        iccid: row.try_get(offset + 3)?,
        model: row.try_get(offset + 4)?,
        firmware: row.try_get(offset + 5)?,
        protocol: row.try_get(offset + 6)?,
        vehicle_id: row.try_get(offset + 7)?,
        status: row.try_get(offset + 8)?,
        auth_code: row.try_get(offset + 9)?,
        last_heartbeat_at: row.try_get(offset + 10)?,
        created_at: row.try_get(offset + 11)?,
        bound_at: row.try_get(offset + 12)?,
    })
}

pub(crate) const TERMINAL_COLS: &str =
    "t.id, t.imei, t.phone, t.iccid, t.model, t.firmware, t.protocol, \
    t.vehicle_id, t.status, t.auth_code, t.last_heartbeat_at, t.created_at, t.bound_at";

pub(crate) fn map_alert(row: &AnyRow) -> sqlx::Result<Alert> {
    Ok(Alert {
        id: row.try_get(0)?,
        vehicle_id: row.try_get(1)?,
        alert_type: row.try_get(2)?,
        level: row.try_get(3)?,
        title: row.try_get(4)?,
        message: row.try_get(5)?,
        lat: get_opt_f64(row, 6)?,
        lon: get_opt_f64(row, 7)?,
        ts: row.try_get(8)?,
        acknowledged: get_bool(row, 9)?,
        ack_by: row.try_get(10)?,
        ack_at: row.try_get(11)?,
        plate_no: row.try_get(12)?,
        model: row.try_get(13)?,
    })
}

pub(crate) const ALERT_COLS: &str =
    "a.id, a.vehicle_id, a.alert_type, a.level, a.title, a.message, \
    a.lat, a.lon, a.ts, a.acknowledged, a.ack_by, a.ack_at, v.plate_no, v.model";

pub(crate) fn map_command(row: &AnyRow) -> sqlx::Result<Command> {
    Ok(Command {
        id: row.try_get(0)?,
        vehicle_id: row.try_get(1)?,
        command: row.try_get(2)?,
        status: row.try_get(3)?,
        requested_by: row.try_get(4)?,
        payload: row.try_get(5)?,
        result: row.try_get(6)?,
        created_at: row.try_get(7)?,
        sent_at: row.try_get(8)?,
        completed_at: row.try_get(9)?,
        plate_no: row.try_get(10)?,
        model: row.try_get(11)?,
    })
}

pub(crate) const COMMAND_COLS: &str = "c.id, c.vehicle_id, c.command, c.status, c.requested_by, \
    c.payload, c.result, c.created_at, c.sent_at, c.completed_at, v.plate_no, v.model";

pub(crate) fn map_trip(row: &AnyRow) -> sqlx::Result<Trip> {
    Ok(Trip {
        id: row.try_get(0)?,
        vehicle_id: row.try_get(1)?,
        start_ts: row.try_get(2)?,
        end_ts: row.try_get(3)?,
        distance_km: get_f64(row, 4)?,
        avg_speed: get_f64(row, 5)?,
        max_speed: get_f64(row, 6)?,
        start_lat: get_f64(row, 7)?,
        start_lon: get_f64(row, 8)?,
        end_lat: get_opt_f64(row, 9)?,
        end_lon: get_opt_f64(row, 10)?,
        status: row.try_get(11)?,
        plate_no: row.try_get(12)?,
        model: row.try_get(13)?,
    })
}

pub(crate) const TRIP_COLS: &str = "t.id, t.vehicle_id, t.start_ts, t.end_ts, t.distance_km, \
    t.avg_speed, t.max_speed, t.start_lat, t.start_lon, t.end_lat, t.end_lon, t.status, \
    v.plate_no, v.model";

pub(crate) fn map_geofence(row: &AnyRow) -> sqlx::Result<Geofence> {
    let polygon_raw: Option<String> = row.try_get(6)?;
    let polygon = polygon_raw.and_then(|raw| serde_json::from_str::<Vec<[f64; 2]>>(&raw).ok());
    Ok(Geofence {
        id: row.try_get(0)?,
        name: row.try_get(1)?,
        kind: row.try_get(2)?,
        center_lat: get_f64(row, 3)?,
        center_lon: get_f64(row, 4)?,
        radius_m: get_f64(row, 5)?,
        polygon,
        color: row.try_get(7)?,
        active: get_bool(row, 8)?,
        created_at: row.try_get(9)?,
        org_name: row.try_get(10)?,
        inside_count: 0,
    })
}

pub(crate) const GEOFENCE_COLS: &str =
    "g.id, g.name, g.kind, g.center_lat, g.center_lon, g.radius_m, g.polygon, g.color, \
     g.active, g.created_at, g.org_name";

pub(crate) fn map_telemetry(row: &AnyRow) -> sqlx::Result<TelemetryPoint> {
    Ok(TelemetryPoint {
        id: get_i64(row, 0)?,
        vehicle_id: row.try_get(1)?,
        ts: row.try_get(2)?,
        lat: get_f64(row, 3)?,
        lon: get_f64(row, 4)?,
        speed: get_f64(row, 5)?,
        heading: get_f64(row, 6)?,
        ignition: get_bool(row, 7)?,
        soc: get_f64(row, 8)?,
        odometer_km: get_f64(row, 9)?,
        source: row.try_get(10)?,
    })
}

pub(crate) const TELEMETRY_COLS: &str = "id, vehicle_id, ts, lat, lon, speed, heading, ignition, \
    soc, odometer_km, source";
