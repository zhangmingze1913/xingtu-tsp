//! Trip state machine for location ingestion.

use sqlx::{Any, Row, Transaction};

use crate::{error::AppResult, models::LocationSample, util::minutes_between};

use super::{get_f64, Db};

struct ActiveTrip {
    id: String,
    start_ts: String,
    distance_km: f64,
    max_speed: f64,
}

impl Db {
    pub(crate) async fn update_trip(
        &self,
        tx: &mut Transaction<'_, Any>,
        sample: &LocationSample,
        gps_distance_km: f64,
    ) -> AppResult<(bool, bool)> {
        let sql = self.sql(
            "SELECT id, start_ts, distance_km, max_speed FROM trips
             WHERE vehicle_id = ?1 AND status = 'active'
             ORDER BY start_ts DESC LIMIT 1",
        );
        let active_trip = self
            .query(&sql)
            .bind(sample.vehicle_id.as_str())
            .fetch_optional(&mut *tx)
            .await?;
        let active_trip = match active_trip {
            Some(row) => Some(ActiveTrip {
                id: row.try_get(0)?,
                start_ts: row.try_get(1)?,
                distance_km: get_f64(&row, 2)?,
                max_speed: get_f64(&row, 3)?,
            }),
            None => None,
        };

        let mut trip_started = false;
        let mut trip_ended = false;

        if sample.ignition && active_trip.is_none() {
            let sql = self.sql(
                "INSERT INTO trips
                 (id, vehicle_id, start_ts, end_ts, distance_km, avg_speed, max_speed,
                  start_lat, start_lon, end_lat, end_lon, status)
                 VALUES (?1, ?2, ?3, NULL, 0, 0, ?4, ?5, ?6, NULL, NULL, 'active')",
            );
            self.query(&sql)
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(sample.vehicle_id.as_str())
                .bind(sample.ts.as_str())
                .bind(sample.speed)
                .bind(sample.lat)
                .bind(sample.lon)
                .execute(&mut *tx)
                .await?;
            trip_started = true;
        } else if let Some(trip) = active_trip {
            if sample.ignition {
                let distance = trip.distance_km + gps_distance_km;
                let max_speed = trip.max_speed.max(sample.speed);
                let elapsed_hours = (minutes_between(&trip.start_ts, &sample.ts) / 60.0).max(0.0);
                let avg_speed = if elapsed_hours > 0.001 {
                    (distance / elapsed_hours).min(200.0)
                } else {
                    0.0
                };
                let sql = self.sql(
                    "UPDATE trips SET distance_km = ?2, max_speed = ?3, avg_speed = ?4,
                        end_lat = ?5, end_lon = ?6
                     WHERE id = ?1",
                );
                self.query(&sql)
                    .bind(&trip.id)
                    .bind(distance)
                    .bind(max_speed)
                    .bind(avg_speed)
                    .bind(sample.lat)
                    .bind(sample.lon)
                    .execute(&mut *tx)
                    .await?;
            } else {
                let elapsed_hours = (minutes_between(&trip.start_ts, &sample.ts) / 60.0).max(0.0);
                let avg_speed = if elapsed_hours > 0.001 {
                    (trip.distance_km / elapsed_hours).min(200.0)
                } else {
                    0.0
                };
                let sql = self.sql(
                    "UPDATE trips SET end_ts = ?2, end_lat = ?3, end_lon = ?4,
                        avg_speed = ?5, status = 'completed'
                     WHERE id = ?1",
                );
                self.query(&sql)
                    .bind(&trip.id)
                    .bind(sample.ts.as_str())
                    .bind(sample.lat)
                    .bind(sample.lon)
                    .bind(avg_speed)
                    .execute(&mut *tx)
                    .await?;
                trip_ended = true;
            }
        }
        Ok((trip_started, trip_ended))
    }
}
