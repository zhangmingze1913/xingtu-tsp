//! Persisted TDengine/PostgreSQL reconciliation results.

use sqlx::Row;

use crate::{error::AppResult, models::TimeseriesReconciliation};

use super::Db;

impl Db {
    pub async fn record_timeseries_reconciliation(
        &self,
        vehicle_id: &str,
        from_ts: &str,
        to_ts: &str,
        pg_count: i64,
        td_count: i64,
        count_delta: i64,
    ) -> AppResult<String> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let checked_at = crate::util::now_iso();
        self.query(
            "INSERT INTO timeseries_reconciliations
             (id, vehicle_id, from_ts, to_ts, pg_count, td_count, count_delta, consistent, checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&id)
        .bind(vehicle_id)
        .bind(from_ts)
        .bind(to_ts)
        .bind(pg_count)
        .bind(td_count)
        .bind(count_delta)
        .bind(i64::from(count_delta == 0))
        .bind(&checked_at)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn list_timeseries_reconciliations(
        &self,
        limit: i64,
    ) -> AppResult<Vec<TimeseriesReconciliation>> {
        let rows = self
            .query(
                "SELECT id, vehicle_id, from_ts, to_ts, pg_count, td_count,
                        count_delta, consistent, checked_at
                 FROM timeseries_reconciliations
                 ORDER BY checked_at DESC LIMIT ?1",
            )
            .bind(limit.clamp(1, 200))
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .iter()
            .map(|row| TimeseriesReconciliation {
                id: row.get::<String, _>(0),
                vehicle_id: row.get::<String, _>(1),
                from_ts: row.get::<String, _>(2),
                to_ts: row.get::<String, _>(3),
                pg_count: row.get::<i64, _>(4),
                td_count: row.get::<i64, _>(5),
                count_delta: row.get::<i64, _>(6),
                consistent: row.get::<i64, _>(7) != 0,
                checked_at: row.get::<String, _>(8),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reconciliation_history_roundtrip() {
        let db = Db::in_memory().await.expect("db");
        let id = db
            .record_timeseries_reconciliation(
                "veh-1",
                "2026-11-24T00:00:00Z",
                "2026-11-24T01:00:00Z",
                10,
                8,
                2,
            )
            .await
            .expect("record");
        let rows = db.list_timeseries_reconciliations(10).await.expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].count_delta, 2);
        assert!(!rows[0].consistent);
    }
}
