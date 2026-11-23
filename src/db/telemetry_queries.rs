//! Telemetry reads and lifecycle updates.

use std::collections::HashMap;

use sqlx::Row;

use crate::{
    error::AppResult,
    models::TelemetryPoint,
    util::{iso_from_now_minutes, now_iso},
};

use super::{get_i64, map_telemetry, Db, TELEMETRY_COLS};

impl Db {
    pub async fn telemetry_exists(&self, vehicle_id: &str, ts: &str) -> AppResult<bool> {
        let sql = self.sql("SELECT COUNT(*) FROM telemetry WHERE vehicle_id = ?1 AND ts = ?2");
        let row = self
            .query(&sql)
            .bind(vehicle_id)
            .bind(ts)
            .fetch_one(self.pool())
            .await?;
        Ok(get_i64(&row, 0)? > 0)
    }

    /// Existing timestamps for one vehicle, used to pre-filter a 0x0704 batch
    /// with a single `IN (...)` query instead of one query per point.
    pub async fn existing_telemetry_timestamps(
        &self,
        vehicle_id: &str,
        timestamps: &[String],
    ) -> AppResult<std::collections::HashSet<String>> {
        use std::collections::HashSet;

        let mut existing = HashSet::new();
        if timestamps.is_empty() {
            return Ok(existing);
        }
        let mut sql = String::from("SELECT ts FROM telemetry WHERE vehicle_id = ?1 AND ts IN (");
        for index in 0..timestamps.len() {
            if index > 0 {
                sql.push(',');
            }
            sql.push_str(&format!("?{}", index + 2));
        }
        sql.push(')');
        let query = self.query(&sql);
        let mut query = query.bind(vehicle_id);
        for ts in timestamps {
            query = query.bind(ts.as_str());
        }
        let rows = query.fetch_all(self.pool()).await?;
        for row in &rows {
            existing.insert(row.try_get(0)?);
        }
        Ok(existing)
    }

    pub async fn trajectory(
        &self,
        vehicle_id: &str,
        from: Option<String>,
        to: Option<String>,
        limit: i64,
    ) -> crate::error::AppResult<Vec<TelemetryPoint>> {
        let from = from.unwrap_or_else(|| iso_from_now_minutes(-120));
        let to = to.unwrap_or_else(now_iso);
        let limit = limit.clamp(1, 2000);
        let query = format!(
            "SELECT {TELEMETRY_COLS} FROM (
                SELECT {TELEMETRY_COLS} FROM telemetry
                WHERE vehicle_id = ?1 AND ts >= ?2 AND ts <= ?3
                ORDER BY ts DESC LIMIT ?4
             ) AS recent ORDER BY ts ASC"
        );
        let sql = self.sql(&query);
        let rows = self
            .query(&sql)
            .bind(vehicle_id)
            .bind(&from)
            .bind(&to)
            .bind(limit)
            .fetch_all(self.pool())
            .await?;
        let mut points = Vec::with_capacity(rows.len());
        for row in &rows {
            points.push(map_telemetry(row)?);
        }
        Ok(points)
    }

    pub async fn recent_telemetry(
        &self,
        vehicle_id: &str,
        limit: i64,
    ) -> crate::error::AppResult<Vec<TelemetryPoint>> {
        let limit = limit.clamp(1, 500);
        let query = format!(
            "SELECT {TELEMETRY_COLS} FROM (
                SELECT {TELEMETRY_COLS} FROM telemetry WHERE vehicle_id = ?1
                ORDER BY ts DESC LIMIT ?2
             ) AS recent ORDER BY ts ASC"
        );
        let sql = self.sql(&query);
        let rows = self
            .query(&sql)
            .bind(vehicle_id)
            .bind(limit)
            .fetch_all(self.pool())
            .await?;
        let mut points = Vec::with_capacity(rows.len());
        for row in &rows {
            points.push(map_telemetry(row)?);
        }
        Ok(points)
    }

    /// Mark terminals and their vehicles offline when no heartbeat/location has
    /// arrived before `cutoff`. Returns (terminals_updated, vehicles_updated).
    pub async fn mark_stale_offline(
        &self,
        cutoff: &str,
        now: &str,
    ) -> crate::error::AppResult<(u64, u64)> {
        let sql = self.sql(
            "UPDATE terminals SET status = 'offline'
             WHERE status = 'online'
               AND (last_heartbeat_at IS NULL OR last_heartbeat_at < ?1)",
        );
        let terminals = self
            .query(&sql)
            .bind(cutoff)
            .execute(self.pool())
            .await?
            .rows_affected();

        let sql = self.sql(
            "UPDATE vehicle_state SET status = 'offline', updated_at = ?2
             WHERE status NOT IN ('offline', 'maintenance')
               AND vehicle_id IN (
                   SELECT vehicle_id FROM terminals
                   WHERE vehicle_id IS NOT NULL
                     AND (last_heartbeat_at IS NULL OR last_heartbeat_at < ?1)
               )",
        );
        let vehicles = self
            .query(&sql)
            .bind(cutoff)
            .bind(now)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok((terminals, vehicles))
    }

    /// Touch only the simulator's demo vehicles, never a whole tenant/fleet.
    pub async fn update_terminal_heartbeats_for_vehicles(
        &self,
        vehicle_ids: &[String],
    ) -> crate::error::AppResult<()> {
        if vehicle_ids.is_empty() {
            return Ok(());
        }
        let ts = now_iso();
        let mut sql = String::from(
            "UPDATE terminals SET status = 'online', last_heartbeat_at = ?1
             WHERE vehicle_id IN (",
        );
        for index in 0..vehicle_ids.len() {
            if index > 0 {
                sql.push(',');
            }
            sql.push_str(&format!("?{}", index + 2));
        }
        sql.push(')');
        let mut query = self.query(&sql).bind(&ts);
        for id in vehicle_ids {
            query = query.bind(id.as_str());
        }
        query.execute(self.pool()).await?;
        Ok(())
    }

    pub async fn vehicle_status_counts(&self) -> crate::error::AppResult<HashMap<String, i64>> {
        let rows = sqlx::query(
            "SELECT COALESCE(s.status, 'offline'), COUNT(*) FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id GROUP BY 1",
        )
        .fetch_all(self.pool())
        .await?;
        let mut counts = HashMap::with_capacity(rows.len());
        for row in &rows {
            counts.insert(row.try_get::<String, _>(0)?, get_i64(row, 1)?);
        }
        Ok(counts)
    }

    /// Remove telemetry older than `cutoff` (RFC3339). Used by the startup prune.
    pub async fn prune_telemetry_before(&self, cutoff: &str) -> AppResult<u64> {
        let sql = self.sql("DELETE FROM telemetry WHERE ts < ?1");
        let result = self.query(&sql).bind(cutoff).execute(self.pool()).await?;
        Ok(result.rows_affected())
    }
}
