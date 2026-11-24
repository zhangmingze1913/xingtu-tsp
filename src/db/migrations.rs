//! Schema bootstrap and versioned migrations.

use sqlx::Row;

use crate::error::AppResult;

use super::schema::{MYSQL_SCHEMA, POSTGRES_SCHEMA, SQLITE_SCHEMA};
use super::sql::split_sql_statements;
use super::{Backend, Db};

const SCHEMA_LOCK_ID: i64 = 8_120_261_116;

impl Db {
    pub(crate) async fn initialize(&self) -> AppResult<()> {
        let schema = match self.backend {
            Backend::Sqlite => SQLITE_SCHEMA,
            Backend::Postgres => POSTGRES_SCHEMA,
            Backend::Mysql => MYSQL_SCHEMA,
        };
        match self.backend {
            Backend::Sqlite => {
                sqlx::raw_sql(schema).execute(&self.pool).await?;
            }
            Backend::Postgres => {
                // Concurrent replicas must not race on CREATE TABLE. Take a
                // transaction-scoped advisory lock so DDL is serialized.
                let mut tx = self.pool.begin().await?;
                sqlx::query("SELECT pg_advisory_xact_lock($1)")
                    .bind(SCHEMA_LOCK_ID)
                    .execute(&mut *tx)
                    .await?;
                sqlx::raw_sql(schema).execute(&mut *tx).await?;
                tx.commit().await?;
            }
            Backend::Mysql => {
                // MySQL's prepared-statement protocol rejects multi-statement
                // SQL, so run each DDL statement on its own. `CREATE TABLE IF
                // NOT EXISTS` keeps bootstrap idempotent.
                let mut connection = self.pool.acquire().await?;
                for statement in split_sql_statements(schema) {
                    sqlx::query(statement).execute(&mut *connection).await?;
                }
            }
        }
        self.ensure_migration_table().await?;
        self.query(
            "INSERT INTO schema_migrations (version, applied_at)
             SELECT ?1, ?2
             WHERE NOT EXISTS (SELECT 1 FROM schema_migrations WHERE version = ?1)",
        )
        .bind("0001_baseline")
        .bind(crate::util::now_iso())
        .execute(self.pool())
        .await?;
        self.apply_migration(
            "0002_users_org_scope",
            "ALTER TABLE users ADD COLUMN org_name VARCHAR(128) NOT NULL DEFAULT ''",
        )
        .await?;
        self.apply_migration(
            "0003_geofences_org_scope",
            "ALTER TABLE geofences ADD COLUMN org_name VARCHAR(128) NOT NULL DEFAULT ''",
        )
        .await?;
        self.apply_migration(
            "0004_vehicles_org_index",
            "CREATE INDEX idx_vehicles_org ON vehicles(org_name)",
        )
        .await?;
        self.apply_migration(
            "0005_geofences_org_index",
            "CREATE INDEX idx_geofences_org ON geofences(org_name)",
        )
        .await?;
        self.apply_migration(
            "0006_terminals_heartbeat_index",
            "CREATE INDEX idx_terminals_heartbeat ON terminals(status, last_heartbeat_at)",
        )
        .await?;
        self.apply_migration(
            "0009_vehicles_org_plate_index",
            "CREATE INDEX idx_vehicles_org_plate ON vehicles(org_name, plate_no)",
        )
        .await?;
        self.apply_migration(
            "0010_refresh_tokens",
            "CREATE TABLE IF NOT EXISTS refresh_tokens (
                 id VARCHAR(64) PRIMARY KEY,
                 user_id VARCHAR(64) NOT NULL,
                 token_hash VARCHAR(64) NOT NULL UNIQUE,
                 expires_at VARCHAR(64) NOT NULL,
                 revoked_at VARCHAR(64),
                 created_at VARCHAR(64) NOT NULL
             )",
        )
        .await?;
        self.apply_migration(
            "0011_refresh_tokens_user_index",
            "CREATE INDEX idx_refresh_tokens_user ON refresh_tokens(user_id)",
        )
        .await?;
        self.apply_migration(
            "0012_telemetry_extra_json",
            "ALTER TABLE telemetry ADD COLUMN extra_json TEXT",
        )
        .await?;
        self.apply_migration(
            "0013_terminal_profiles",
            "CREATE TABLE IF NOT EXISTS terminal_profiles (
                 terminal_id VARCHAR(64) PRIMARY KEY,
                 parameters_json TEXT,
                 attributes_json TEXT,
                 updated_at VARCHAR(40) NOT NULL
             )",
        )
        .await?;
        self.apply_migration(
            "0014_terminal_registration_json",
            "ALTER TABLE terminal_profiles ADD COLUMN registration_json TEXT",
        )
        .await?;
        self.apply_migration(
            "0015_telemetry_claims",
            "CREATE TABLE IF NOT EXISTS telemetry_claims (
                 vehicle_id VARCHAR(64) NOT NULL,
                 ts VARCHAR(40) NOT NULL,
                 created_at VARCHAR(40) NOT NULL,
                 PRIMARY KEY (vehicle_id, ts)
             )",
        )
        .await?;
        self.apply_migration(
            "0016_terminal_parameter_versions",
            "CREATE TABLE IF NOT EXISTS terminal_parameter_versions (
                 id VARCHAR(64) PRIMARY KEY,
                 terminal_id VARCHAR(64) NOT NULL,
                 version INTEGER NOT NULL,
                 parameters_json TEXT NOT NULL,
                 created_at VARCHAR(40) NOT NULL
             )",
        )
        .await?;
        self.apply_migration(
            "0018_key_audit",
            "CREATE TABLE IF NOT EXISTS key_audit (
                 id VARCHAR(64) PRIMARY KEY,
                 provider VARCHAR(64) NOT NULL,
                 action VARCHAR(32) NOT NULL,
                 success INTEGER NOT NULL,
                 created_at VARCHAR(40) NOT NULL
             )",
        )
        .await?;
        self.apply_migration(
            "0017_pii_vault",
            "CREATE TABLE IF NOT EXISTS pii_vault (
                 id VARCHAR(64) PRIMARY KEY,
                 owner_type VARCHAR(64) NOT NULL,
                 owner_id VARCHAR(64) NOT NULL,
                 field VARCHAR(64) NOT NULL,
                 ciphertext TEXT NOT NULL,
                 updated_at VARCHAR(40) NOT NULL
             )",
        )
        .await?;
        let index_sql = if self.backend == Backend::Mysql {
            "CREATE INDEX idx_pii_vault_owner ON pii_vault (owner_type, owner_id, field)"
        } else {
            "CREATE INDEX IF NOT EXISTS idx_pii_vault_owner
             ON pii_vault (owner_type, owner_id, field)"
        };
        self.apply_migration(
            "0019_gb32960_devices",
            "CREATE TABLE IF NOT EXISTS gb32960_devices (
                 vin VARCHAR(17) PRIMARY KEY,
                 enabled INTEGER NOT NULL,
                 note VARCHAR(200) NOT NULL,
                 created_at VARCHAR(40) NOT NULL
             )",
        )
        .await?;
        self.apply_migration("0018_pii_vault_index", index_sql)
            .await?;
        self.apply_migration(
            "0020_timeseries_reconciliations",
            "CREATE TABLE IF NOT EXISTS timeseries_reconciliations (
                 id VARCHAR(64) PRIMARY KEY,
                 vehicle_id VARCHAR(64) NOT NULL,
                 from_ts VARCHAR(40) NOT NULL,
                 to_ts VARCHAR(40) NOT NULL,
                 pg_count BIGINT NOT NULL,
                 td_count BIGINT NOT NULL,
                 count_delta BIGINT NOT NULL,
                 consistent INTEGER NOT NULL,
                 checked_at VARCHAR(40) NOT NULL
             )",
        )
        .await?;
        self.apply_migration(
            "0021_timeseries_reconciliations_time_index",
            "CREATE INDEX idx_timeseries_reconciliations_time
             ON timeseries_reconciliations(checked_at)",
        )
        .await?;
        self.apply_migration(
            "0022_timeseries_reconciliations_org",
            "ALTER TABLE timeseries_reconciliations ADD COLUMN org_name VARCHAR(128) NOT NULL DEFAULT ''",
        )
        .await?;
        Ok(())
    }

    async fn ensure_migration_table(&self) -> AppResult<()> {
        if self.backend == Backend::Postgres {
            let mut tx = self.pool.begin().await?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(SCHEMA_LOCK_ID)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                     version TEXT PRIMARY KEY,
                     applied_at TEXT NOT NULL
                 )",
            )
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        } else {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                     version TEXT PRIMARY KEY,
                     applied_at TEXT NOT NULL
                 )",
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn apply_migration(&self, version: &str, sql: &str) -> AppResult<()> {
        let exists = self
            .query("SELECT COUNT(*) FROM schema_migrations WHERE version = ?1")
            .bind(version)
            .fetch_one(self.pool())
            .await?;
        if exists.try_get::<i64, _>(0)? > 0 {
            return Ok(());
        }
        let result = sqlx::query(sql).execute(&self.pool).await;
        if let Err(error) = &result {
            // Tolerate a manually pre-added column: only the duplicate-column
            // error is ignored, then the migration is recorded.
            let text = error.to_string().to_ascii_lowercase();
            if !(text.contains("duplicate column")
                || text.contains("already exists")
                || text.contains("duplicate key name")
                || text.contains("pg_type_typname_nsp_index")
                || text.contains("duplicate key value violates unique constraint"))
            {
                result?;
            }
        }
        let insert = if self.is_mysql() {
            "INSERT INTO schema_migrations (version, applied_at)
             VALUES (?1, ?2)
             ON DUPLICATE KEY UPDATE version = version"
        } else {
            "INSERT INTO schema_migrations (version, applied_at)
             VALUES (?1, ?2)
             ON CONFLICT(version) DO NOTHING"
        };
        self.query(insert)
            .bind(version)
            .bind(crate::util::now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn schema_versions(&self) -> AppResult<Vec<String>> {
        let rows = sqlx::query("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|row| row.try_get::<String, _>(0))
            .collect::<Result<Vec<_>, _>>()?)
    }
}
