//! Telemetry ingestion and state transitions.
//!
//! A single 0x0200 point and a 0x0704 batch share `ingest_location_tx`; the
//! batch path sorts chronologically and commits once. `claim` is true for
//! batches so a retransmitted batch cannot insert the same (vehicle, ts)
//! twice. Alert dedupe and trip/fence transitions stay in the same transaction
//! as the point, trading per-point queries for exactly-once business state.

use std::collections::HashMap;

use sqlx::{Any, Row, Transaction};

use crate::{
    error::AppResult,
    models::{Alert, IngestOutcome, LocationSample},
    util::haversine_m,
};

use super::{get_f64, get_i64, get_opt_f64, Db};

#[derive(Clone)]
pub(crate) struct PreviousState {
    pub(crate) lat: Option<f64>,
    pub(crate) lon: Option<f64>,
    pub(crate) speed: f64,
    pub(crate) soc: f64,
    pub(crate) odometer_km: f64,
    pub(crate) status: String,
    pub(crate) updated_at: String,
}

/// Per-batch vehicle metadata plus the last accepted state. The batch path
/// advances this in memory instead of re-reading vehicle_state for every point.
struct VehicleContext {
    plate_no: String,
    model: String,
    previous: Option<PreviousState>,
}

impl Db {
    /// Load vehicle metadata and its previous state once per batch vehicle.
    async fn load_vehicle_context(
        &self,
        tx: &mut Transaction<'_, Any>,
        vehicle_id: &str,
    ) -> AppResult<Option<VehicleContext>> {
        let sql = self.sql(
            "SELECT v.plate_no, v.model,
                    s.lat, s.lon, s.speed, s.soc, s.odometer_km, s.status, s.updated_at
             FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
             WHERE v.id = ?1",
        );
        let Some(row) = self
            .query(&sql)
            .bind(vehicle_id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(None);
        };
        let previous = if row.try_get::<Option<String>, _>(8)?.is_some() {
            Some(PreviousState {
                lat: get_opt_f64(&row, 2)?,
                lon: get_opt_f64(&row, 3)?,
                speed: get_f64(&row, 4)?,
                soc: get_f64(&row, 5)?,
                odometer_km: get_f64(&row, 6)?,
                status: row.try_get(7)?,
                updated_at: row.try_get(8)?,
            })
        } else {
            None
        };
        Ok(Some(VehicleContext {
            plate_no: row.try_get(0)?,
            model: row.try_get(1)?,
            previous,
        }))
    }

    /// Insert an alert inside the ingestion transaction. Kept as a method so the
    /// backend-specific placeholder translation is shared with every other query.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn insert_alert_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        vehicle_id: &str,
        plate_no: &str,
        model: &str,
        alert_type: &str,
        level: &str,
        title: &str,
        message: String,
        lat: f64,
        lon: f64,
        ts: &str,
    ) -> AppResult<Alert> {
        let id = uuid::Uuid::new_v4().to_string();
        let sql = self.sql(
            "INSERT INTO alerts
             (id, vehicle_id, alert_type, level, title, message, lat, lon, ts, acknowledged)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
        );
        self.query(&sql)
            .bind(&id)
            .bind(vehicle_id)
            .bind(alert_type)
            .bind(level)
            .bind(title)
            .bind(message.as_str())
            .bind(lat)
            .bind(lon)
            .bind(ts)
            .execute(&mut *tx)
            .await?;
        Ok(Alert {
            id,
            vehicle_id: vehicle_id.to_string(),
            alert_type: alert_type.to_string(),
            level: level.to_string(),
            title: title.to_string(),
            message,
            lat: Some(lat),
            lon: Some(lon),
            ts: ts.to_string(),
            acknowledged: false,
            ack_by: None,
            ack_at: None,
            plate_no: Some(plate_no.to_string()),
            model: Some(model.to_string()),
        })
    }

    pub(crate) async fn has_recent_alert_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        vehicle_id: &str,
        alert_type: &str,
        since: &str,
    ) -> AppResult<bool> {
        let sql = self.sql(
            "SELECT COUNT(*) FROM alerts
             WHERE vehicle_id = ?1 AND alert_type = ?2 AND ts >= ?3",
        );
        let row = self
            .query(&sql)
            .bind(vehicle_id)
            .bind(alert_type)
            .bind(since)
            .fetch_one(&mut *tx)
            .await?;
        Ok(get_i64(&row, 0)? > 0)
    }

    /// Claim a (vehicle, timestamp) key inside the ingestion transaction.
    /// Concurrent/retransmitted batches can only claim a key once. The claim
    /// rolls back with the business transaction on failure.
    async fn claim_telemetry_tx(
        &self,
        tx: &mut Transaction<'_, Any>,
        vehicle_id: &str,
        ts: &str,
    ) -> AppResult<bool> {
        let sql = if self.is_mysql() {
            "INSERT IGNORE INTO telemetry_claims (vehicle_id, ts, created_at)
             VALUES (?1, ?2, ?3)"
        } else {
            "INSERT INTO telemetry_claims (vehicle_id, ts, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(vehicle_id, ts) DO NOTHING"
        };
        let result = self
            .query(sql)
            .bind(vehicle_id)
            .bind(ts)
            .bind(crate::util::now_iso())
            .execute(&mut *tx)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Backfill claims from existing telemetry once, so a restart does not
    /// re-admit historical timestamps. Safe to call repeatedly.
    pub async fn count_telemetry_for(&self, vehicle_id: &str) -> AppResult<i64> {
        let row = self
            .query("SELECT COUNT(*) FROM telemetry WHERE vehicle_id = ?1")
            .bind(vehicle_id)
            .fetch_one(self.pool())
            .await?;
        Ok(get_i64(&row, 0)?)
    }

    pub async fn backfill_telemetry_claims(&self) -> AppResult<u64> {
        let count = self
            .query("SELECT COUNT(*) FROM telemetry_claims")
            .fetch_one(self.pool())
            .await?
            .try_get::<i64, _>(0)?;
        if count > 0 {
            return Ok(0);
        }
        let sql = if self.is_mysql() {
            "INSERT IGNORE INTO telemetry_claims (vehicle_id, ts, created_at)
             SELECT DISTINCT vehicle_id, ts, ?1 FROM telemetry"
        } else {
            "INSERT INTO telemetry_claims (vehicle_id, ts, created_at)
             SELECT DISTINCT vehicle_id, ts, ?1 FROM telemetry
             ON CONFLICT(vehicle_id, ts) DO NOTHING"
        };
        let result = self
            .query(sql)
            .bind(crate::util::now_iso())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn ingest_location(
        &self,
        sample: LocationSample,
    ) -> crate::error::AppResult<Option<IngestOutcome>> {
        let mut tx = self.pool().begin().await?;
        let outcome = self
            .ingest_location_tx(&mut tx, sample, false, None)
            .await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Batch variant: one transaction for all samples, used by JT/T 808 0x0704.
    pub async fn ingest_locations(
        &self,
        mut samples: Vec<LocationSample>,
    ) -> crate::error::AppResult<Vec<Option<IngestOutcome>>> {
        // 0x0704 batches can arrive out of timestamp order; trip/fence state
        // machines need chronological input.
        samples.sort_by(|a, b| a.ts.cmp(&b.ts));
        let mut tx = self.pool().begin().await?;
        // One vehicle/state read per distinct vehicle, not per point. The
        // in-memory previous state advances with each accepted sample.
        let mut contexts: HashMap<String, VehicleContext> = HashMap::new();
        for sample in &samples {
            if !contexts.contains_key(&sample.vehicle_id) {
                if let Some(context) = self
                    .load_vehicle_context(&mut tx, &sample.vehicle_id)
                    .await?
                {
                    contexts.insert(sample.vehicle_id.clone(), context);
                }
            }
        }
        let mut outcomes = Vec::with_capacity(samples.len());
        for sample in samples {
            let vehicle_id = sample.vehicle_id.clone();
            let Some(context) = contexts.get(&vehicle_id) else {
                outcomes.push(None);
                continue;
            };
            let outcome = self
                .ingest_location_tx(&mut tx, sample, true, Some(context))
                .await?;
            if let Some(outcome) = &outcome {
                if let Some(context) = contexts.get_mut(&vehicle_id) {
                    context.previous = Some(PreviousState {
                        lat: Some(outcome.lat),
                        lon: Some(outcome.lon),
                        speed: outcome.speed,
                        soc: outcome.soc,
                        odometer_km: outcome.odometer_km,
                        status: outcome.status.clone(),
                        updated_at: outcome.ts.clone(),
                    });
                }
            }
            outcomes.push(outcome);
        }
        tx.commit().await?;
        Ok(outcomes)
    }

    async fn ingest_location_tx(
        &self,
        tx: &mut Transaction<'_, Any>,
        sample: LocationSample,
        claim: bool,
        prefetched: Option<&VehicleContext>,
    ) -> crate::error::AppResult<Option<IngestOutcome>> {
        let (plate_no, model, previous) = if let Some(context) = prefetched {
            (
                context.plate_no.clone(),
                context.model.clone(),
                context.previous.clone(),
            )
        } else {
            let Some(context) = self.load_vehicle_context(tx, &sample.vehicle_id).await? else {
                return Ok(None);
            };
            (context.plate_no, context.model, context.previous)
        };
        if claim
            && !self
                .claim_telemetry_tx(tx, &sample.vehicle_id, &sample.ts)
                .await?
        {
            return Ok(None);
        }

        let prev_soc = previous.as_ref().map(|p| p.soc).unwrap_or(0.0);
        let prev_odometer = previous.as_ref().map(|p| p.odometer_km).unwrap_or(0.0);
        let prev_lat_lon = previous
            .as_ref()
            .and_then(|p| p.lat.zip(p.lon))
            .filter(|(lat, lon)| *lat != 0.0 && *lon != 0.0);

        let gps_distance_km = prev_lat_lon
            .map(|(lat, lon)| haversine_m(lat, lon, sample.lat, sample.lon) / 1000.0)
            .filter(|distance| *distance < 5.0)
            .unwrap_or(0.0);

        let soc = sample.soc.unwrap_or(prev_soc).clamp(0.0, 100.0);
        let odometer_km = sample
            .odometer_km
            .unwrap_or(prev_odometer + gps_distance_km);
        let status = if sample.speed > 1.0 && sample.ignition {
            "moving"
        } else if sample.ignition {
            "idle"
        } else if previous.as_ref().map(|p| p.status.as_str()) == Some("charging") {
            "charging"
        } else {
            "idle"
        };

        let sql = if self.postgis_enabled() {
            self.sql(
                "INSERT INTO telemetry
                 (vehicle_id, ts, lat, lon, speed, heading, ignition, soc, odometer_km, source, extra_json, geom)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                         ST_SetSRID(ST_MakePoint(?4, ?3), 4326))",
            )
        } else {
            self.sql(
                "INSERT INTO telemetry
                 (vehicle_id, ts, lat, lon, speed, heading, ignition, soc, odometer_km, source, extra_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
        };
        self.query(&sql)
            .bind(sample.vehicle_id.as_str())
            .bind(sample.ts.as_str())
            .bind(sample.lat)
            .bind(sample.lon)
            .bind(sample.speed)
            .bind(sample.heading)
            .bind(sample.ignition as i64)
            .bind(soc)
            .bind(odometer_km)
            .bind(sample.source.as_str())
            .bind(sample.extra_json.as_deref())
            .execute(&mut *tx)
            .await?;

        let sql = if self.postgis_enabled() {
            self.sql(
                "UPDATE vehicle_state SET
                    lat = ?2, lon = ?3, speed = ?4, heading = ?5, ignition = ?6,
                    soc = ?7, odometer_km = ?8, status = ?9, updated_at = ?10,
                    geom = ST_SetSRID(ST_MakePoint(?3, ?2), 4326)
                 WHERE vehicle_id = ?1",
            )
        } else {
            self.sql(
                "UPDATE vehicle_state SET
                    lat = ?2, lon = ?3, speed = ?4, heading = ?5, ignition = ?6,
                    soc = ?7, odometer_km = ?8, status = ?9, updated_at = ?10
                 WHERE vehicle_id = ?1",
            )
        };
        self.query(&sql)
            .bind(sample.vehicle_id.as_str())
            .bind(sample.lat)
            .bind(sample.lon)
            .bind(sample.speed)
            .bind(sample.heading)
            .bind(sample.ignition as i64)
            .bind(soc)
            .bind(odometer_km)
            .bind(status)
            .bind(sample.ts.as_str())
            .execute(&mut *tx)
            .await?;

        let (trip_started, trip_ended) =
            self.update_trip(&mut *tx, &sample, gps_distance_km).await?;

        let alerts = self
            .detect_alerts(&mut *tx, &sample, &previous, &plate_no, &model, soc)
            .await?;

        Ok(Some(IngestOutcome {
            vehicle_id: sample.vehicle_id,
            plate_no,
            ts: sample.ts,
            lat: sample.lat,
            lon: sample.lon,
            speed: sample.speed,
            heading: sample.heading,
            ignition: sample.ignition,
            soc,
            odometer_km,
            status: status.to_string(),
            alerts,
            trip_started,
            trip_ended,
        }))
    }
}
