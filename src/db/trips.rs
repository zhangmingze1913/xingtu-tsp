use sqlx::Row;

use crate::{
    error::AppResult,
    models::{Page, TelemetryPoint, Trip},
    util::page_bounds,
};

use super::{get_i64, map_telemetry, map_trip, Db, TELEMETRY_COLS, TRIP_COLS};

impl Db {
    pub async fn list_trips(
        &self,
        vehicle_id: Option<String>,
        page: Option<i64>,
        page_size: Option<i64>,
        org: &str,
    ) -> AppResult<Page<Trip>> {
        let (page, page_size, offset) = page_bounds(page, page_size);
        let vehicle_id = vehicle_id.unwrap_or_default();

        let count_sql = self.sql(
            "SELECT COUNT(*) FROM trips t
             JOIN vehicles v ON v.id = t.vehicle_id
             WHERE (?1 = '' OR t.vehicle_id = ?1)
               AND (?2 = '*' OR v.org_name = ?2)",
        );
        let count_row = self
            .query(&count_sql)
            .bind(vehicle_id.as_str())
            .bind(org)
            .fetch_one(self.pool())
            .await?;
        let total = get_i64(&count_row, 0)?;

        let query = format!(
            "SELECT {TRIP_COLS} FROM trips t
             JOIN vehicles v ON v.id = t.vehicle_id
             WHERE (?1 = '' OR t.vehicle_id = ?1)
               AND (?2 = '*' OR v.org_name = ?2)
             ORDER BY t.start_ts DESC LIMIT ?3 OFFSET ?4"
        );
        let sql = self.sql(&query);
        let rows = self
            .query(&sql)
            .bind(vehicle_id.as_str())
            .bind(org)
            .bind(page_size)
            .bind(offset)
            .fetch_all(self.pool())
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push(map_trip(row)?);
        }
        Ok(Page {
            items,
            total,
            page,
            page_size,
        })
    }

    pub async fn get_trip(&self, id: &str) -> AppResult<Option<Trip>> {
        let query = format!(
            "SELECT {TRIP_COLS} FROM trips t
             JOIN vehicles v ON v.id = t.vehicle_id WHERE t.id = ?1"
        );
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_trip(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn trip_points(&self, trip_id: &str) -> AppResult<Vec<TelemetryPoint>> {
        let sql = self.sql("SELECT start_ts, end_ts FROM trips WHERE id = ?1");
        let row = self
            .query(&sql)
            .bind(trip_id)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(Vec::new());
        };
        let start_ts: String = row.try_get(0)?;
        let end_ts: Option<String> = row.try_get(1)?;
        let end_ts = end_ts.unwrap_or_else(crate::util::now_iso);
        let query = format!(
            "SELECT {TELEMETRY_COLS} FROM telemetry t
             WHERE t.vehicle_id = (SELECT vehicle_id FROM trips WHERE id = ?1)
               AND t.ts >= ?2 AND t.ts <= ?3
             ORDER BY t.ts ASC LIMIT 2000"
        );
        let sql = self.sql(&query);
        let rows = self
            .query(&sql)
            .bind(trip_id)
            .bind(&start_ts)
            .bind(&end_ts)
            .fetch_all(self.pool())
            .await?;
        let mut points = Vec::with_capacity(rows.len());
        for row in &rows {
            points.push(map_telemetry(row)?);
        }
        Ok(points)
    }

    pub async fn active_trip(&self, vehicle_id: &str) -> AppResult<Option<Trip>> {
        let query = format!(
            "SELECT {TRIP_COLS} FROM trips t
             JOIN vehicles v ON v.id = t.vehicle_id
             WHERE t.vehicle_id = ?1 AND t.status = 'active'
             ORDER BY t.start_ts DESC LIMIT 1"
        );
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(vehicle_id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_trip(&row)?)),
            None => Ok(None),
        }
    }
}
