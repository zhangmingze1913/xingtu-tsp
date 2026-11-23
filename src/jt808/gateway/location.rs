//! Location report/batch ingestion.

use super::*;

pub(super) async fn ingest_location(
    state: &AppState,
    frame: &Frame,
    location: &LocationData,
    authenticated: bool,
    peer: &str,
    sessions: &mut HashSet<String>,
    _out_tx: &mpsc::UnboundedSender<Vec<u8>>,
) -> u8 {
    if !authenticated {
        tracing::debug!(terminal = %frame.terminal_id, "location before authentication rejected");
        return 1;
    }
    let terminal = state
        .db
        .get_terminal_by_phone(&frame.terminal_id)
        .await
        .ok()
        .flatten();
    let Some(terminal) = terminal else {
        tracing::debug!(terminal = %frame.terminal_id, "location from unknown terminal");
        return 1;
    };
    let Some(vehicle_id) = terminal.vehicle_id else {
        tracing::debug!(terminal = %frame.terminal_id, "terminal is not bound to a vehicle");
        return 1;
    };
    sessions.insert(frame.terminal_id.clone());
    state
        .cluster
        .register_session(&frame.terminal_id, peer)
        .await;
    if let Err(error) = state.db.heartbeat_terminal(&frame.terminal_id).await {
        tracing::warn!(%error, "JT808 heartbeat update failed");
    }
    let sample = LocationSample {
        vehicle_id,
        ts: if location.ts.is_empty() {
            now_iso()
        } else {
            location.ts.clone()
        },
        lat: location.lat,
        lon: location.lon,
        speed: location.speed,
        heading: location.heading,
        ignition: location.ignition,
        soc: None,
        odometer_km: location.mileage_km,
        source: "jt808".to_string(),
        extra_json: location.extra_json.clone(),
    };
    if state
        .db
        .telemetry_exists(&sample.vehicle_id, &sample.ts)
        .await
        .unwrap_or(false)
    {
        state
            .metrics
            .telemetry_duplicates
            .fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    match services::telemetry::ingest(state, sample).await {
        Ok(Some(_)) => {
            tracing::debug!(terminal = %frame.terminal_id, "JT808 location ingested");
            0
        }
        Ok(None) => 1,
        Err(error) => {
            tracing::warn!(%error, "JT808 location ingest failed");
            2
        }
    }
}

pub(super) async fn ingest_batch(
    state: &AppState,
    frame: &Frame,
    locations: &[LocationData],
    authenticated: bool,
    peer: &str,
    sessions: &mut HashSet<String>,
    _out_tx: &mpsc::UnboundedSender<Vec<u8>>,
) -> u8 {
    if !authenticated {
        return 1;
    }
    let Some(terminal) = state
        .db
        .get_terminal_by_phone(&frame.terminal_id)
        .await
        .ok()
        .flatten()
    else {
        return 1;
    };
    let Some(vehicle_id) = terminal.vehicle_id else {
        return 1;
    };
    sessions.insert(frame.terminal_id.clone());
    state
        .cluster
        .register_session(&frame.terminal_id, peer)
        .await;
    if let Err(error) = state.db.heartbeat_terminal(&frame.terminal_id).await {
        tracing::warn!(%error, "JT808 heartbeat update failed");
    }

    let timestamps: Vec<String> = locations
        .iter()
        .map(|location| {
            if location.ts.is_empty() {
                now_iso()
            } else {
                location.ts.clone()
            }
        })
        .collect();
    let existing = state
        .db
        .existing_telemetry_timestamps(&vehicle_id, &timestamps)
        .await
        .unwrap_or_default();
    let mut samples = Vec::with_capacity(locations.len());
    let mut seen = HashSet::new();
    for (location, ts) in locations.iter().zip(timestamps) {
        if !seen.insert(ts.clone()) {
            state
                .metrics
                .telemetry_duplicates
                .fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if existing.contains(&ts) {
            state
                .metrics
                .telemetry_duplicates
                .fetch_add(1, Ordering::Relaxed);
            continue;
        }
        samples.push(LocationSample {
            vehicle_id: vehicle_id.clone(),
            ts,
            lat: location.lat,
            lon: location.lon,
            speed: location.speed,
            heading: location.heading,
            ignition: location.ignition,
            soc: None,
            odometer_km: location.mileage_km,
            source: "jt808".to_string(),
            extra_json: location.extra_json.clone(),
        });
    }
    if samples.is_empty() {
        return 0;
    }
    match services::telemetry::ingest_batch(state, samples).await {
        Ok(outcomes) if outcomes.iter().any(Option::is_some) => 0,
        Ok(_) => 1,
        Err(error) => {
            tracing::warn!(%error, "JT808 batch ingest failed");
            2
        }
    }
}
