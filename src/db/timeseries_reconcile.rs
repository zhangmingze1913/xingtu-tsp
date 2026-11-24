//! Persisted TDengine/PostgreSQL reconciliation results.

use sqlx::Row;

use crate::{error::AppResult, models::TimeseriesReconciliation};

use super::Db;

impl Db {
    pub async fn vehicle_org_name(&self, vehicle_id: &str) -> AppResult<Option<String>> {
        let row = self
            .query("SELECT org_name FROM vehicles WHERE id = ?1")
            .bind(vehicle_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|row| row.get::<String, _>(0)))
    }

    pub async fn record_timeseries_reconciliation(
        &self,
        vehicle_id: &str,
        org_name: &str,
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
             (id, vehicle_id, org_name, from_ts, to_ts, pg_count, td_count, count_delta, consistent, checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(&id)
        .bind(vehicle_id)
        .bind(org_name)
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
        vehicle_id: Option<&str>,
        org_name: Option<&str>,
        consistent: Option<bool>,
    ) -> AppResult<Vec<TimeseriesReconciliation>> {
        let rows = self
            .query(
                "SELECT id, vehicle_id, org_name, from_ts, to_ts, pg_count, td_count,
                        count_delta, consistent, checked_at
                 FROM timeseries_reconciliations
                 WHERE (?1 IS NULL OR vehicle_id = ?1)
                   AND (?2 IS NULL OR org_name = ?2)
                   AND (?3 IS NULL OR consistent = ?3)
                 ORDER BY checked_at DESC LIMIT ?4",
            )
            .bind(vehicle_id)
            .bind(org_name)
            .bind(consistent.map(i64::from))
            .bind(limit.clamp(1, 200))
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .iter()
            .map(|row| TimeseriesReconciliation {
                id: row.get::<String, _>(0),
                vehicle_id: row.get::<String, _>(1),
                org_name: row.get::<String, _>(2),
                from_ts: row.get::<String, _>(3),
                to_ts: row.get::<String, _>(4),
                pg_count: row.get::<i64, _>(5),
                td_count: row.get::<i64, _>(6),
                count_delta: row.get::<i64, _>(7),
                consistent: row.get::<i64, _>(8) != 0,
                checked_at: row.get::<String, _>(9),
            })
            .collect())
    }
    pub async fn prune_timeseries_reconciliations_before(&self, cutoff: &str) -> AppResult<u64> {
        let result = self
            .query("DELETE FROM timeseries_reconciliations WHERE checked_at < ?1")
            .bind(cutoff)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
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
                "Org-1",
                "2026-11-24T00:00:00Z",
                "2026-11-24T01:00:00Z",
                10,
                8,
                2,
            )
            .await
            .expect("record");
        let rows = db
            .list_timeseries_reconciliations(10, None, None, None)
            .await
            .expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].count_delta, 2);
        assert!(!rows[0].consistent);
    }

    #[tokio::test]
    async fn reconciliation_history_prunes_old_rows() {
        let db = Db::in_memory().await.expect("db");
        db.query(
            "INSERT INTO timeseries_reconciliations
             (id, vehicle_id, org_name, from_ts, to_ts, pg_count, td_count, count_delta, consistent, checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind("old")
        .bind("veh-old")
        .bind("")
        .bind("a")
        .bind("b")
        .bind(1i64)
        .bind(0i64)
        .bind(1i64)
        .bind(0i64)
        .bind("2000-01-01T00:00:00Z")
        .execute(db.pool())
        .await
        .expect("insert");
        let pruned = db
            .prune_timeseries_reconciliations_before("2020-01-01T00:00:00Z")
            .await
            .expect("prune");
        assert_eq!(pruned, 1);
        assert!(db
            .list_timeseries_reconciliations(10, None, None, None)
            .await
            .expect("list")
            .is_empty());
    }
}
