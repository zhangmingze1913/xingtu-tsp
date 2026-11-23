//! Demo dataset schema and small deterministic helpers.

use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct DemoData {
    pub(super) vehicles: Vec<DemoVehicle>,
    pub(super) geofences: Vec<DemoFence>,
    pub(super) alerts: Vec<DemoAlert>,
    pub(super) commands: Vec<DemoCommand>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DemoVehicle {
    pub(super) plate: String,
    pub(super) model: String,
    pub(super) energy: String,
    pub(super) color: String,
    pub(super) driver: String,
    pub(super) phone: String,
    pub(super) org: String,
    pub(super) lat: f64,
    pub(super) lon: f64,
    pub(super) status: String,
    pub(super) soc: f64,
    pub(super) odometer: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct DemoFence {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) center_lat: f64,
    pub(super) center_lon: f64,
    pub(super) radius_m: f64,
    pub(super) polygon: Option<Vec<[f64; 2]>>,
    pub(super) color: String,
    pub(super) active: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct DemoAlert {
    pub(super) vehicle_index: usize,
    pub(super) alert_type: String,
    pub(super) level: String,
    pub(super) title: String,
    pub(super) message: String,
    pub(super) minutes_ago: i64,
    pub(super) acknowledged: bool,
    pub(super) ack_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DemoCommand {
    pub(super) vehicle_index: usize,
    pub(super) command: String,
    pub(super) status: String,
    pub(super) result: String,
    pub(super) minutes_ago: i64,
    pub(super) requested_by: String,
}

#[derive(Debug, Clone)]
pub(super) struct Point {
    pub(super) vehicle_id: String,
    pub(super) ts: String,
    pub(super) lat: f64,
    pub(super) lon: f64,
    pub(super) speed: f64,
    pub(super) heading: f64,
    pub(super) ignition: bool,
    pub(super) soc: f64,
    pub(super) odometer_km: f64,
    pub(super) source: String,
}

pub(super) struct SeedTrip {
    pub(super) id: String,
    pub(super) vehicle_id: String,
    pub(super) start_ts: String,
    pub(super) end_ts: Option<String>,
    pub(super) distance_km: f64,
    pub(super) avg_speed: f64,
    pub(super) max_speed: f64,
    pub(super) start_lat: f64,
    pub(super) start_lon: f64,
    pub(super) end_lat: Option<f64>,
    pub(super) end_lon: Option<f64>,
    pub(super) status: String,
}

pub(super) struct SeedTerminal {
    pub(super) id: String,
    pub(super) imei: String,
    pub(super) phone: String,
    pub(super) iccid: String,
    pub(super) model: String,
    pub(super) vehicle_id: String,
    pub(super) status: String,
    pub(super) auth_code: String,
    pub(super) last_heartbeat_at: Option<String>,
    pub(super) created_at: String,
    pub(super) bound_at: String,
}

pub(super) fn iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(super) fn shift(lat: f64, lon: f64, meters: f64, heading_deg: f64) -> (f64, f64) {
    let rad = heading_deg.to_radians();
    let d_lat = meters * rad.cos() / 111_320.0;
    let d_lon = meters * rad.sin() / (111_320.0 * lat.to_radians().cos());
    (lat + d_lat, lon + d_lon)
}
