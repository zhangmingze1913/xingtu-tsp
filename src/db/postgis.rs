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
    let extensions: &[&str] = if db.is_kingbase() {
        &["kingbase_spatial", "postgis"]
    } else {
        &["postgis"]
    };
    let mut installed = false;
    for extension in extensions {
        let count = sqlx::query("SELECT COUNT(*) FROM pg_extension WHERE extname = $1")
            .bind(extension)
            .fetch_one(&db.pool)
            .await?
            .try_get::<i64, _>(0)?;
        if count > 0 {
            installed = true;
            break;
        }
    }
    if !installed {
        for extension in extensions {
            let sql = format!("CREATE EXTENSION IF NOT EXISTS {extension}");
            match sqlx::query(&sql).execute(&db.pool).await {
                Ok(_) => {
                    installed = true;
                    break;
                }
                Err(error) => {
                    tracing::warn!(%error, extension, "spatial extension is not installed or not permitted")
                }
            }
        }
        if !installed {
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
        "#,
    )
    .execute(&db.pool)
    .await?;

    let geography_indexes = r#"
        CREATE INDEX IF NOT EXISTS idx_telemetry_geog
            ON telemetry USING GIST ((geom::geography));
        CREATE INDEX IF NOT EXISTS idx_vehicle_state_geog
            ON vehicle_state USING GIST ((geom::geography));
    "#;
    if let Err(error) = sqlx::raw_sql(geography_indexes).execute(&db.pool).await {
        if db.is_kingbase() {
            tracing::warn!(%error, "KGIS geography indexes unavailable; geometry indexes remain");
        } else {
            return Err(error.into());
        }
    }

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
    .bind(if db.is_kingbase() {
        "0007_kgis_geometry"
    } else {
        POSTGIS_MIGRATION
    })
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
