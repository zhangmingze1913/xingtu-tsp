//! Optional PostGIS integration.
//!
//! PostgreSQL gets `geometry(Point,4326)` columns, GiST indexes and a
//! backfill. If the extension is unavailable the rest of the application keeps
//! using the portable lat/lon columns.

use sqlx::Row;

use crate::error::AppResult;

use super::{Backend, Db};

const POSTGIS_MIGRATION: &str = "0007_postgis_geometry";

pub(crate) async fn enable(db: &Db) -> AppResult<bool> {
    if db.backend != Backend::Postgres {
        return Ok(false);
    }

    let installed = sqlx::query("SELECT COUNT(*) FROM pg_extension WHERE extname = 'postgis'")
        .fetch_one(&db.pool)
        .await?
        .try_get::<i64, _>(0)?
        > 0;
    if !installed {
        if let Err(error) = sqlx::query("CREATE EXTENSION IF NOT EXISTS postgis")
            .execute(&db.pool)
            .await
        {
            tracing::warn!(%error, "PostGIS extension is not installed or not permitted");
            return Ok(false);
        }
    }

    let has_geom = sqlx::query(
        "SELECT COUNT(*) FROM information_schema.columns
         WHERE table_name = 'telemetry' AND column_name = 'geom'",
    )
    .fetch_one(&db.pool)
    .await?
    .try_get::<i64, _>(0)?
        > 0;

    sqlx::raw_sql(
        r#"
        ALTER TABLE telemetry
            ADD COLUMN IF NOT EXISTS geom geometry(Point, 4326);
        ALTER TABLE vehicle_state
            ADD COLUMN IF NOT EXISTS geom geometry(Point, 4326);

        CREATE INDEX IF NOT EXISTS idx_telemetry_geom
            ON telemetry USING GIST (geom);
        CREATE INDEX IF NOT EXISTS idx_vehicle_state_geom
            ON vehicle_state USING GIST (geom);
        CREATE INDEX IF NOT EXISTS idx_telemetry_geog
            ON telemetry USING GIST ((geom::geography));
        CREATE INDEX IF NOT EXISTS idx_vehicle_state_geog
            ON vehicle_state USING GIST ((geom::geography));
        "#,
    )
    .execute(&db.pool)
    .await?;

    if !has_geom {
        sqlx::raw_sql(
            r#"
            UPDATE telemetry
            SET geom = ST_SetSRID(ST_MakePoint(lon, lat), 4326)
            WHERE geom IS NULL AND lon IS NOT NULL AND lat IS NOT NULL;
            UPDATE vehicle_state
            SET geom = ST_SetSRID(ST_MakePoint(lon, lat), 4326)
            WHERE geom IS NULL AND lon IS NOT NULL AND lat IS NOT NULL;
            "#,
        )
        .execute(&db.pool)
        .await?;
    }

    db.query(
        "INSERT INTO schema_migrations (version, applied_at)
         VALUES (?1, ?2)
         ON CONFLICT(version) DO NOTHING",
    )
    .bind(POSTGIS_MIGRATION)
    .bind(crate::util::now_iso())
    .execute(db.pool())
    .await?;
    Ok(true)
}

impl Db {
    /// Backfill rows inserted by seed data after PostGIS was enabled.
    pub(crate) async fn backfill_postgis(&self) -> AppResult<()> {
        if !self.postgis_enabled() {
            return Ok(());
        }
        sqlx::raw_sql(
            r#"
            UPDATE telemetry
            SET geom = ST_SetSRID(ST_MakePoint(lon, lat), 4326)
            WHERE geom IS NULL AND lon IS NOT NULL AND lat IS NOT NULL;
            UPDATE vehicle_state
            SET geom = ST_SetSRID(ST_MakePoint(lon, lat), 4326)
            WHERE geom IS NULL AND lon IS NOT NULL AND lat IS NOT NULL;
            "#,
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }
}
