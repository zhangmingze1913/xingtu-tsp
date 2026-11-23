mod dashboard;
pub use dashboard::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub org_name: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// Backward-compatible alias of `access_token`.
    pub token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub user: User,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vehicle {
    pub id: String,
    pub vin: String,
    pub plate_no: String,
    pub model: String,
    pub color: String,
    pub energy_type: String,
    pub driver_name: String,
    pub driver_phone: String,
    pub org_name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleState {
    pub vehicle_id: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub speed: f64,
    pub heading: f64,
    pub ignition: bool,
    pub soc: f64,
    pub odometer_km: f64,
    pub status: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Terminal {
    pub id: String,
    pub imei: String,
    pub phone: String,
    pub iccid: String,
    pub model: String,
    pub firmware: String,
    pub protocol: String,
    pub vehicle_id: Option<String>,
    pub status: String,
    #[serde(skip_serializing)]
    pub auth_code: Option<String>,
    pub last_heartbeat_at: Option<String>,
    pub created_at: String,
    pub bound_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleDetail {
    #[serde(flatten)]
    pub vehicle: Vehicle,
    pub state: VehicleState,
    pub terminal: Option<Terminal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryPoint {
    pub id: i64,
    pub vehicle_id: String,
    pub ts: String,
    pub lat: f64,
    pub lon: f64,
    pub speed: f64,
    pub heading: f64,
    pub ignition: bool,
    pub soc: f64,
    pub odometer_km: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trip {
    pub id: String,
    pub vehicle_id: String,
    pub start_ts: String,
    pub end_ts: Option<String>,
    pub distance_km: f64,
    pub avg_speed: f64,
    pub max_speed: f64,
    pub start_lat: f64,
    pub start_lon: f64,
    pub end_lat: Option<f64>,
    pub end_lon: Option<f64>,
    pub status: String,
    pub plate_no: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Geofence {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub center_lat: f64,
    pub center_lon: f64,
    pub radius_m: f64,
    pub polygon: Option<Vec<[f64; 2]>>,
    pub color: String,
    pub active: bool,
    pub created_at: String,
    #[serde(default)]
    pub org_name: String,
    #[serde(default)]
    pub inside_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub vehicle_id: String,
    pub alert_type: String,
    pub level: String,
    pub title: String,
    pub message: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub ts: String,
    pub acknowledged: bool,
    pub ack_by: Option<String>,
    pub ack_at: Option<String>,
    pub plate_no: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub vehicle_id: String,
    pub command: String,
    pub status: String,
    pub requested_by: String,
    pub payload: Option<String>,
    pub result: Option<String>,
    pub created_at: String,
    pub sent_at: Option<String>,
    pub completed_at: Option<String>,
    pub plate_no: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: i64,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub action: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct VehicleListQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVehicleRequest {
    pub vin: String,
    pub plate_no: String,
    pub model: String,
    #[serde(default)]
    pub color: String,
    #[serde(default = "default_energy")]
    pub energy_type: String,
    #[serde(default)]
    pub driver_name: String,
    #[serde(default)]
    pub driver_phone: String,
    #[serde(default)]
    pub org_name: String,
}

fn default_energy() -> String {
    "EV".to_string()
}

#[derive(Debug, Deserialize)]
pub struct UpdateVehicleRequest {
    pub plate_no: Option<String>,
    pub model: Option<String>,
    pub color: Option<String>,
    pub energy_type: Option<String>,
    pub driver_name: Option<String>,
    pub driver_phone: Option<String>,
    pub org_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrajectoryQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub vehicle_id: String,
    pub ts: Option<String>,
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub heading: f64,
    #[serde(default)]
    pub ignition: bool,
    pub soc: Option<f64>,
    pub odometer_km: Option<f64>,
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "api".to_string()
}

#[derive(Debug, Clone)]
pub struct LocationSample {
    pub vehicle_id: String,
    pub ts: String,
    pub lat: f64,
    pub lon: f64,
    pub speed: f64,
    pub heading: f64,
    pub ignition: bool,
    pub soc: Option<f64>,
    pub odometer_km: Option<f64>,
    pub source: String,
    /// JSON map of JT808 0x0200 additional items (id -> hex value).
    pub extra_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestOutcome {
    pub vehicle_id: String,
    pub plate_no: String,
    pub ts: String,
    pub lat: f64,
    pub lon: f64,
    pub speed: f64,
    pub heading: f64,
    pub ignition: bool,
    pub soc: f64,
    pub odometer_km: f64,
    pub status: String,
    pub alerts: Vec<Alert>,
    pub trip_started: bool,
    pub trip_ended: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateGeofenceRequest {
    pub name: String,
    #[serde(default = "default_circle")]
    pub kind: String,
    pub center_lat: Option<f64>,
    pub center_lon: Option<f64>,
    #[serde(default = "default_radius")]
    pub radius_m: f64,
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_circle() -> String {
    "circle".to_string()
}
fn default_radius() -> f64 {
    800.0
}
fn default_color() -> String {
    "#22d3ee".to_string()
}
fn default_active() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct UpdateGeofenceRequest {
    pub name: Option<String>,
    pub radius_m: Option<f64>,
    pub color: Option<String>,
    pub active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct AlertListQuery {
    pub status: Option<String>,
    pub level: Option<String>,
    pub vehicle_id: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AckRequest {
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommandRequest {
    pub command: String,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CommandListQuery {
    pub status: Option<String>,
    pub vehicle_id: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct BindTerminalRequest {
    pub vehicle_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TripListQuery {
    pub vehicle_id: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct BindVehicleTerminalRequest {
    pub imei: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub iccid: String,
    #[serde(default = "default_terminal_model")]
    pub model: String,
}

fn default_terminal_model() -> String {
    "XT-TBOX-5G".to_string()
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub database: bool,
    pub cluster_enabled: bool,
    pub cluster_leader: bool,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct TerminalRow {
    #[serde(flatten)]
    pub terminal: Terminal,
    pub plate_no: Option<String>,
}
