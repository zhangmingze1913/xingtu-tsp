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

    #[allow(clippy::too_many_arguments)]
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
        self.list_timeseries_reconciliations_page(limit, None, vehicle_id, org_name, consistent)
            .await
    }

    pub async fn list_timeseries_reconciliations_page(
        &self,
        limit: i64,
        before: Option<(&str, &str)>,
        vehicle_id: Option<&str>,
        org_name: Option<&str>,
        consistent: Option<bool>,
    ) -> AppResult<Vec<TimeseriesReconciliation>> {
        let (before_ts, before_id) = match before {
            Some((ts, id)) => (Some(ts), Some(id)),
            None => (None, None),
        };
        let rows = self
            .query(
                "SELECT id, vehicle_id, org_name, from_ts, to_ts, pg_count, td_count,
                        count_delta, consistent, checked_at
                 FROM timeseries_reconciliations
                 WHERE (?1 IS NULL OR vehicle_id = ?1)
                   AND (?2 IS NULL OR org_name = ?2)
                   AND (?3 IS NULL OR consistent = ?3)
                   AND (?4 IS NULL OR checked_at < ?4
                        OR (checked_at = ?4 AND id < ?5))
                 ORDER BY checked_at DESC, id DESC LIMIT ?6",
            )
            .bind(vehicle_id)
            .bind(org_name)
            .bind(consistent.map(i64::from))
            .bind(before_ts)
            .bind(before_id)
            .bind(limit.clamp(1, 5000))
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
    pub async fn create_reconciliation_export_job(
        &self,
        org_name: &str,
        vehicle_id: Option<&str>,
        consistent: Option<bool>,
        max_rows: i64,
    ) -> AppResult<String> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let now = crate::util::now_iso();
        self.query(
            "INSERT INTO reconciliation_export_jobs
             (id, status, org_name, vehicle_id, consistent, max_rows, rows_written, created_at, updated_at)
             VALUES (?1, 'running', ?2, ?3, ?4, ?5, 0, ?6, ?6)",
        )
        .bind(&id)
        .bind(org_name)
        .bind(vehicle_id)
        .bind(consistent.map(i64::from))
        .bind(max_rows.clamp(1, 5000))
        .bind(&now)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn update_reconciliation_export_job(
        &self,
        id: &str,
        status: &str,
        rows_written: i64,
        file_path: Option<&str>,
        error: Option<&str>,
    ) -> AppResult<()> {
        self.query(
            "UPDATE reconciliation_export_jobs
             SET status = ?1, rows_written = ?2, file_path = ?3, error = ?4, updated_at = ?5
             WHERE id = ?6",
        )
        .bind(status)
        .bind(rows_written)
        .bind(file_path)
        .bind(error)
        .bind(crate::util::now_iso())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn get_reconciliation_export_job(
        &self,
        id: &str,
    ) -> AppResult<Option<crate::models::ReconciliationExportJob>> {
        let row = self
            .query(
                "SELECT id, status, org_name, vehicle_id, consistent, max_rows,
                        rows_written, error, created_at, updated_at
                 FROM reconciliation_export_jobs WHERE id = ?1",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|row| crate::models::ReconciliationExportJob {
            id: row.get::<String, _>(0),
            status: row.get::<String, _>(1),
            org_name: row.get::<String, _>(2),
            vehicle_id: row.get::<Option<String>, _>(3),
            consistent: row.get::<Option<i64>, _>(4).map(|value| value != 0),
            max_rows: row.get::<i64, _>(5),
            rows_written: row.get::<i64, _>(6),
            error: row.get::<Option<String>, _>(7),
            created_at: row.get::<String, _>(8),
            updated_at: row.get::<String, _>(9),
        }))
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

    #[tokio::test]
    async fn reconciliation_history_keyset_pagination() {
        let db = Db::in_memory().await.expect("db");
        for (id, checked) in [
            ("a", "2026-11-24T00:00:00Z"),
            ("b", "2026-11-24T00:00:01Z"),
            ("c", "2026-11-24T00:00:02Z"),
        ] {
            db.query(
                "INSERT INTO timeseries_reconciliations
                 (id, vehicle_id, org_name, from_ts, to_ts, pg_count, td_count, count_delta, consistent, checked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .bind(id)
            .bind("veh")
            .bind("org")
            .bind("f")
            .bind("t")
            .bind(1i64)
            .bind(0i64)
            .bind(1i64)
            .bind(0i64)
            .bind(checked)
            .execute(db.pool())
            .await
            .expect("insert");
        }
        let first = db
            .list_timeseries_reconciliations_page(2, None, None, None, None)
            .await
            .expect("first");
        assert_eq!(first.len(), 2);
        let last = first.last().unwrap();
        let second = db
            .list_timeseries_reconciliations_page(
                2,
                Some((&last.checked_at, &last.id)),
                None,
                None,
                None,
            )
            .await
            .expect("second");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, "a");
    }

    #[tokio::test]
    async fn export_job_lifecycle() {
        let db = Db::in_memory().await.expect("db");
        let id = db
            .create_reconciliation_export_job("Org", Some("veh"), Some(false), 100)
            .await
            .expect("create");
        let job = db
            .get_reconciliation_export_job(&id)
            .await
            .expect("get")
            .expect("job");
        assert_eq!(job.status, "running");
        assert_eq!(job.rows_written, 0);
        db.update_reconciliation_export_job(&id, "completed", 10, Some("/tmp/x.csv"), None)
            .await
            .expect("update");
        let job = db
            .get_reconciliation_export_job(&id)
            .await
            .expect("get")
            .expect("job");
        assert_eq!(job.status, "completed");
        assert_eq!(job.rows_written, 10);
    }
}
