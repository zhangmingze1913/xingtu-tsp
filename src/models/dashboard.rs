//! Dashboard and map read models.

use serde::{Deserialize, Serialize};

use super::Alert;

#[derive(Debug, Serialize)]
pub struct FleetStats {
    pub total: i64,
    pub online: i64,
    pub moving: i64,
    pub idle: i64,
    pub charging: i64,
    pub offline: i64,
    pub maintenance: i64,
    pub online_rate: f64,
    pub avg_soc: f64,
    pub total_mileage_km: f64,
    pub terminal_online_rate: f64,
}

#[derive(Debug, Serialize)]
pub struct AlertStats {
    pub active: i64,
    pub critical: i64,
    pub warning: i64,
    pub today: i64,
}

#[derive(Debug, Serialize)]
pub struct DailyPoint {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Serialize)]
pub struct NameValue {
    pub name: String,
    pub value: i64,
}

#[derive(Debug, Serialize)]
pub struct CommandStats {
    pub total_7d: i64,
    pub success_rate: f64,
}

#[derive(Debug, Serialize)]
pub struct DashboardOverview {
    pub fleet: FleetStats,
    pub alerts: AlertStats,
    pub mileage_7d: Vec<DailyPoint>,
    pub alert_trend_7d: Vec<DailyPoint>,
    pub status_distribution: Vec<NameValue>,
    pub recent_alerts: Vec<Alert>,
    pub command: CommandStats,
    pub generated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct MapQuery {
    pub status: Option<String>,
    /// Optional `min_lon,min_lat,max_lon,max_lat` bounding box.
    pub bbox: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NearbyQuery {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: Option<f64>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapVehicle {
    pub id: String,
    pub plate_no: String,
    pub model: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub speed: f64,
    pub heading: f64,
    pub status: String,
    pub soc: f64,
    pub ignition: bool,
    pub odometer_km: f64,
}
