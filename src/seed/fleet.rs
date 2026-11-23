//! Demo fleet/trip builders. Kept separate from persistence and fixture lists.

use chrono::{DateTime, Duration, Utc};

use crate::{
    models::{Vehicle, VehicleState},
    util::{haversine_m, Lcg},
};

use super::model::*;

pub(super) struct FleetSeed {
    pub(super) vehicles: Vec<Vehicle>,
    pub(super) states: Vec<VehicleState>,
    pub(super) terminals: Vec<SeedTerminal>,
    pub(super) all_points: Vec<Point>,
    pub(super) active_trips: Vec<SeedTrip>,
}

pub(super) fn build(
    demo: &DemoData,
    rng: &mut Lcg,
    now: DateTime<Utc>,
    created_at: String,
) -> FleetSeed {
    let mut vehicles = Vec::new();
    let mut states = Vec::new();
    let mut terminals = Vec::new();
    let mut all_points = Vec::new();
    let mut active_trips = Vec::new();

    for (index, spec) in demo.vehicles.iter().enumerate() {
        let id = format!("veh_{:03}", index + 1);
        let vehicle = Vehicle {
            id: id.clone(),
            vin: format!("LSVAA2A41NX{:06}", index + 1),
            plate_no: spec.plate.clone(),
            model: spec.model.clone(),
            color: spec.color.clone(),
            energy_type: spec.energy.clone(),
            driver_name: spec.driver.clone(),
            driver_phone: spec.phone.clone(),
            org_name: spec.org.clone(),
            created_at: created_at.clone(),
        };

        let moving = spec.status == "moving";
        let online = !matches!(spec.status.as_str(), "offline" | "maintenance");
        let step_seconds = 45i64;
        let steps = if moving { 22 } else { 1 };
        let mut lat = spec.lat;
        let mut lon = spec.lon;
        let mut heading = ((index * 47 + 23) % 360) as f64;
        let mut speed = if moving {
            18.0 + rng.range(0.0, 28.0)
        } else {
            0.0
        };
        let start_odometer = spec.odometer - if moving { 6.0 } else { 0.0 };
        let start_soc = (spec.soc + if moving { 2.5 } else { 0.0 }).min(100.0);
        let mut trail = Vec::new();
        let mut last_lat = lat;
        let mut last_lon = lon;
        let mut last_ts = now - Duration::minutes(90);

        for step in 0..steps {
            let progress = if steps > 1 {
                step as f64 / (steps - 1) as f64
            } else {
                1.0
            };
            let ts = if moving {
                now - Duration::seconds(((steps - 1 - step) as i64) * step_seconds)
            } else {
                let minutes_ago = match spec.status.as_str() {
                    "offline" => 95 + index as i64 * 17,
                    "maintenance" => 240,
                    "charging" => 6,
                    _ => 8,
                };
                now - Duration::minutes(minutes_ago)
            };
            if step > 0 {
                heading = (heading + rng.range(-24.0, 24.0)).rem_euclid(360.0);
                let meters = speed / 3.6 * step_seconds as f64;
                let (next_lat, next_lon) = shift(lat, lon, meters, heading);
                if (30.70..31.70).contains(&next_lat) && (120.98..122.04).contains(&next_lon) {
                    lat = next_lat;
                    lon = next_lon;
                } else {
                    heading = (heading + 180.0).rem_euclid(360.0);
                }
                speed = (speed + rng.range(-3.5, 4.2)).clamp(8.0, 72.0);
            }
            let odo = start_odometer + (spec.odometer - start_odometer) * progress;
            let soc = start_soc - (start_soc - spec.soc) * progress;
            let point = Point {
                vehicle_id: id.clone(),
                ts: iso(ts),
                lat,
                lon,
                speed,
                heading,
                ignition: moving || spec.status == "idle",
                soc,
                odometer_km: odo,
                source: "jt808".to_string(),
            };
            trail.push(point.clone());
            all_points.push(point);
            last_lat = lat;
            last_lon = lon;
            last_ts = ts;
        }

        if moving {
            let first = trail.first().expect("moving trail");
            let distance: f64 = trail
                .windows(2)
                .map(|pair| haversine_m(pair[0].lat, pair[0].lon, pair[1].lat, pair[1].lon))
                .sum::<f64>()
                / 1000.0;
            active_trips.push(SeedTrip {
                id: uuid::Uuid::new_v4().to_string(),
                vehicle_id: id.clone(),
                start_ts: first.ts.clone(),
                end_ts: None,
                distance_km: distance.max(2.0),
                avg_speed: 24.0 + rng.range(0.0, 10.0),
                max_speed: speed + rng.range(2.0, 14.0),
                start_lat: first.lat,
                start_lon: first.lon,
                end_lat: Some(last_lat),
                end_lon: Some(last_lon),
                status: "active".to_string(),
            });
        }

        states.push(VehicleState {
            vehicle_id: id.clone(),
            lat: Some(last_lat),
            lon: Some(last_lon),
            speed: if moving { speed } else { 0.0 },
            heading,
            ignition: moving || spec.status == "idle",
            soc: spec.soc,
            odometer_km: spec.odometer,
            status: spec.status.clone(),
            updated_at: iso(last_ts),
        });

        terminals.push(SeedTerminal {
            id: format!("term_{:03}", index + 1),
            imei: format!("86123456789{:04}", index + 1),
            phone: format!("01380000{:04}", index + 1),
            iccid: format!("8986001234567890{:04}", index + 1),
            model: if index % 3 == 0 {
                "XT-TBOX-5G".to_string()
            } else {
                "XT-TBOX-4G".to_string()
            },
            vehicle_id: id.clone(),
            status: if online {
                "online".to_string()
            } else {
                "offline".to_string()
            },
            auth_code: format!("XT{:06}", index + 1),
            last_heartbeat_at: Some(if online {
                iso(now - Duration::seconds(12 + index as i64))
            } else {
                iso(now - Duration::minutes(120))
            }),
            created_at: created_at.clone(),
            bound_at: iso(now - Duration::days(100)),
        });

        vehicles.push(vehicle);
    }

    FleetSeed {
        vehicles,
        states,
        terminals,
        all_points,
        active_trips,
    }
}

pub(super) fn trips(
    vehicles: &[Vehicle],
    states: &[VehicleState],
    rng: &mut Lcg,
    now: DateTime<Utc>,
) -> Vec<SeedTrip> {
    let mut trips = Vec::new();
    for day in 1..=6 {
        for slot in 0..3 {
            let index = ((day * 5 + slot * 7) as usize) % vehicles.len();
            let state = &states[index];
            let lat = state.lat.unwrap_or(31.23);
            let lon = state.lon.unwrap_or(121.47);
            let start = now - Duration::days(day) - Duration::minutes(150 + slot * 47);
            let distance = 12.0 + rng.range(0.0, 45.0);
            let avg = 24.0 + rng.range(0.0, 26.0);
            let duration_min = (distance / avg * 60.0).clamp(12.0, 160.0) as i64;
            trips.push(SeedTrip {
                id: uuid::Uuid::new_v4().to_string(),
                vehicle_id: vehicles[index].id.clone(),
                start_ts: iso(start),
                end_ts: Some(iso(start + Duration::minutes(duration_min))),
                distance_km: (distance * 10.0).round() / 10.0,
                avg_speed: (avg * 10.0).round() / 10.0,
                max_speed: avg + 22.0 + rng.range(0.0, 18.0),
                start_lat: lat,
                start_lon: lon,
                end_lat: Some(lat + rng.range(-0.035, 0.035)),
                end_lon: Some(lon + rng.range(-0.045, 0.045)),
                status: "completed".to_string(),
            });
        }
    }
    for slot in 0..4 {
        let index = (slot * 4 + 1) % vehicles.len();
        let state = &states[index];
        let lat = state.lat.unwrap_or(31.23);
        let lon = state.lon.unwrap_or(121.47);
        let start = now - Duration::hours(3 + slot as i64) - Duration::minutes(20);
        let distance = 9.0 + rng.range(0.0, 18.0);
        let avg = 22.0 + rng.range(0.0, 18.0);
        let duration_min = (distance / avg * 60.0).clamp(15.0, 90.0) as i64;
        trips.push(SeedTrip {
            id: uuid::Uuid::new_v4().to_string(),
            vehicle_id: vehicles[index].id.clone(),
            start_ts: iso(start),
            end_ts: Some(iso(start + Duration::minutes(duration_min))),
            distance_km: (distance * 10.0).round() / 10.0,
            avg_speed: (avg * 10.0).round() / 10.0,
            max_speed: avg + 18.0,
            start_lat: lat,
            start_lon: lon,
            end_lat: Some(lat + rng.range(-0.02, 0.02)),
            end_lon: Some(lon + rng.range(-0.03, 0.03)),
            status: "completed".to_string(),
        });
    }
    trips
}
