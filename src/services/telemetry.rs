use crate::{
    error::AppResult,
    models::{IngestOutcome, LocationSample},
    realtime::RealtimeEvent,
    state::AppState,
};

pub async fn ingest(state: &AppState, sample: LocationSample) -> AppResult<Option<IngestOutcome>> {
    let outcome = state.db.ingest_location(sample).await?;
    if let Some(outcome) = &outcome {
        state.dashboard_cache.clear();
        publish(state, outcome);
    }
    Ok(outcome)
}

/// 0x0704 batch ingestion: one database transaction and one cache invalidation.
pub async fn ingest_batch(
    state: &AppState,
    samples: Vec<LocationSample>,
) -> AppResult<Vec<Option<IngestOutcome>>> {
    let outcomes = state.db.ingest_locations(samples).await?;
    if outcomes.iter().any(Option::is_some) {
        state.dashboard_cache.clear();
    }
    for outcome in outcomes.iter().flatten() {
        publish(state, outcome);
    }
    Ok(outcomes)
}

fn publish(state: &AppState, outcome: &IngestOutcome) {
    use std::sync::atomic::Ordering;
    state
        .metrics
        .telemetry_ingested
        .fetch_add(1, Ordering::Relaxed);
    state
        .metrics
        .alerts_created
        .fetch_add(outcome.alerts.len() as u64, Ordering::Relaxed);
    if let Some(tsink) = &state.tsink {
        tsink.enqueue(outcome);
    }
    if let Some(tdengine) = &state.tdengine {
        tdengine.enqueue(outcome);
    }
    state.bus.publish(RealtimeEvent::Telemetry {
        vehicle_id: outcome.vehicle_id.clone(),
        plate_no: outcome.plate_no.clone(),
        ts: outcome.ts.clone(),
        lat: outcome.lat,
        lon: outcome.lon,
        speed: outcome.speed,
        heading: outcome.heading,
        ignition: outcome.ignition,
        soc: outcome.soc,
        odometer_km: outcome.odometer_km,
        status: outcome.status.clone(),
    });
    for alert in &outcome.alerts {
        state.bus.publish(RealtimeEvent::Alert {
            alert: alert.clone(),
        });
    }
}
