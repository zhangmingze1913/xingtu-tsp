//! PostgreSQL-only covering indexes for the trajectory hot path.

use crate::error::AppResult;

use super::{Backend, Db};

const VERSION: &str = "0008_telemetry_cover_index";

pub(crate) async fn ensure(db: &Db) -> AppResult<()> {
    if db.backend != Backend::Postgres {
        return Ok(());
    }
    let sql = if db.is_kingbase() {
        "CREATE INDEX IF NOT EXISTS idx_telemetry_vehicle_ts_cover
         ON telemetry (vehicle_id, ts DESC)"
    } else {
        "CREATE INDEX IF NOT EXISTS idx_telemetry_vehicle_ts_cover
         ON telemetry (vehicle_id, ts DESC)
         INCLUDE (lat, lon, speed, heading, ignition, soc, odometer_km, source)"
    };
    sqlx::raw_sql(sql).execute(&db.pool).await?;
    db.query(
        "INSERT INTO schema_migrations (version, applied_at)
         VALUES (?1, ?2)
         ON CONFLICT(version) DO NOTHING",
    )
    .bind(VERSION)
    .bind(crate::util::now_iso())
    .execute(db.pool())
    .await?;
    Ok(())
}
