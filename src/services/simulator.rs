use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use chrono::Utc;

use crate::{
    models::LocationSample, realtime::RealtimeEvent, services::telemetry, state::AppState,
    util::now_iso,
};

fn shift(lat: f64, lon: f64, meters: f64, heading_deg: f64) -> (f64, f64) {
    let rad = heading_deg.to_radians();
    let d_lat = meters * rad.cos() / 111_320.0;
    let d_lon = meters * rad.sin() / (111_320.0 * lat.to_radians().cos());
    (lat + d_lat, lon + d_lon)
}

/// Optional embedded fleet simulator. It drives the exact same ingestion service
/// as the JT/T 808 gateway, so fences, trips and alerts all exercise production paths.
pub fn start(state: AppState, leader: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !leader.load(Ordering::Relaxed) {
                continue;
            }
            tick += 1;

            let mut moving = match state.db.map_vehicles(Some("moving".to_string()), "*").await {
                Ok(vehicles) => vehicles,
                Err(error) => {
                    tracing::warn!(%error, "simulator could not load moving vehicles");
                    continue;
                }
            };
            if moving.is_empty() {
                // Demo databases may have been offline-swept after a restart;
                // revive a few vehicles so the demo remains observable.
                match state.db.map_vehicles(None, "*").await {
                    Ok(vehicles) => {
                        moving = vehicles
                            .into_iter()
                            .filter(|vehicle| vehicle.status != "maintenance")
                            .take(6)
                            .collect();
                    }
                    Err(error) => {
                        tracing::warn!(%error, "simulator could not load fallback vehicles");
                        continue;
                    }
                }
            }
            if moving.is_empty() {
                continue;
            }

            let take = moving.len().min(3);
            for offset in 0..take {
                let index = ((tick as usize) * 3 + offset * 5) % moving.len();
                let vehicle = &moving[index];
                let (Some(lat), Some(lon)) = (vehicle.lat, vehicle.lon) else {
                    continue;
                };

                let heading =
                    (vehicle.heading + rand::random::<f64>() * 40.0 - 20.0).rem_euclid(360.0);
                let overspeed = rand::random::<f64>() < 0.002;
                let speed = if overspeed {
                    138.0 + rand::random::<f64>() * 18.0
                } else {
                    (vehicle.speed + rand::random::<f64>() * 12.0 - 6.0).clamp(16.0, 78.0)
                };
                let meters = speed / 3.6 * 3.0;
                let mut next_lat = lat;
                let mut next_lon = lon;
                let (candidate_lat, candidate_lon) = shift(lat, lon, meters, heading);
                if (30.72..31.68).contains(&candidate_lat)
                    && (121.00..122.02).contains(&candidate_lon)
                {
                    next_lat = candidate_lat;
                    next_lon = candidate_lon;
                }

                let soc = if vehicle.soc < 6.0 {
                    62.0 + rand::random::<f64>() * 18.0
                } else {
                    (vehicle.soc - 0.03).max(4.0)
                };

                let sample = LocationSample {
                    vehicle_id: vehicle.id.clone(),
                    ts: now_iso(),
                    lat: next_lat,
                    lon: next_lon,
                    speed,
                    heading,
                    ignition: true,
                    soc: Some(soc),
                    odometer_km: Some(vehicle.odometer_km + meters / 1000.0),
                    source: "simulator".to_string(),
                    extra_json: None,
                };

                if let Err(error) = telemetry::ingest(&state, sample).await {
                    tracing::warn!(%error, vehicle_id = %vehicle.id, "simulator ingest failed");
                }
            }

            if tick.is_multiple_of(10) {
                let ids: Vec<String> = moving.iter().map(|vehicle| vehicle.id.clone()).collect();
                let _ = state.db.update_terminal_heartbeats_for_vehicles(&ids).await;
            }

            if tick.is_multiple_of(20) {
                if let Ok(counts) = state.db.vehicle_status_counts().await {
                    let online = ["moving", "idle", "charging", "online"]
                        .iter()
                        .map(|key| counts.get(*key).copied().unwrap_or(0))
                        .sum();
                    let moving_count = counts.get("moving").copied().unwrap_or(0);
                    state.bus.publish(RealtimeEvent::Fleet {
                        online,
                        moving: moving_count,
                        ts: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    });
                }
            }
        }
    });
}
