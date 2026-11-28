//! Async database layer backed by `sqlx::Any`.
//!
//! The same repository methods run against three backends selected by the
//! `TSP_DATABASE` value:
//!
//! * SQLite — a filesystem path (or an explicit `sqlite:` URL) used for local
//!   development, tests and single-node installs.
//! * PostgreSQL — a `postgres://` / `postgresql://` URL used by clustered
//!   deployments.
//! * MySQL / MariaDB — a `mysql://` or `mariadb://` URL used by single-node
//!   installs that already run a MySQL-compatible server.
//!
//! Portability rules used throughout this module:
//!
//! * Booleans / status flags are stored as `INTEGER` (`0` / `1`) and decoded as
//!   `i64`, because `sqlx::Any` only reports `Bool` for real SQLite/Postgres
//!   boolean columns. MySQL's `TINYINT` is not understood by `sqlx::Any`, so
//!   flags must stay `INTEGER` there.
//! * Timestamps are text columns holding RFC3339 strings, so ordering and
//!   range comparisons are lexicographic and identical on every backend. MySQL
//!   uses bounded `VARCHAR` columns for them because indexed `TEXT` columns
//!   would need prefix lengths.
//! * Telemetry / audit ids are `INTEGER PRIMARY KEY AUTOINCREMENT` on SQLite,
//!   `BIGSERIAL PRIMARY KEY` on PostgreSQL and
//!   `BIGINT AUTO_INCREMENT PRIMARY KEY` on MySQL.
//! * Shared SQL uses `?N` (SQLite's native numbered placeholders) and binds
//!   values in numeric order. [`Db::query`] wraps `sqlx` and translates at
//!   execution time: SQLite receives `?N` verbatim, PostgreSQL gets `$N`, and
//!   MySQL (which only accepts `?`) gets each marker rewritten in occurrence
//!   order with the bound values duplicated for repeated markers.
//! * `PRAGMA` statements are only emitted for SQLite, and MySQL-specific
//!   session setup is only emitted for MySQL, in the pool's `after_connect`
//!   hook. No other engine-only SQL appears in shared code.
//!
//! SQLite file databases get WAL, foreign keys and a busy timeout. PostgreSQL
//! relies on the server defaults (foreign keys and transactions are always
//! enforced). MySQL uses InnoDB and `utf8mb4`.

use std::{
    borrow::Cow,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use sqlx::{any::AnyPoolOptions, AnyPool};

use crate::error::{AppError, AppResult};

pub mod alerts;
mod auth_tokens;
pub mod commands;
pub mod dashboard;
pub mod gb32960;
pub mod geofences;
pub mod pii_vault;
pub mod telemetry;
mod telemetry_queries;
mod telemetry_rules;
mod telemetry_trip;
pub mod terminal_profiles;
mod terminals;
mod timeseries_reconcile;
pub mod trips;
pub mod users;
pub mod vehicles;

mod mappers;
pub(crate) use mappers::*;

mod indexes;
mod map;
mod migrations;
mod postgis;
mod schema;
mod seed_lock;
mod spatial;
mod sql;
#[cfg(test)]
use schema::MYSQL_SCHEMA;
#[cfg(test)]
use sql::{mysql_placeholders, rewrite_placeholders, split_sql_statements};
use sql::{mysql_url_from, sqlite_url_from_path, DbQuery};

/// Database engine selected by the connection URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
    Mysql,
}

fn kingbase_url_from(raw: &str) -> String {
    for prefix in ["kingbase8://", "kingbasees://", "kingbase://"] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return format!("postgres://{rest}");
        }
    }
    raw.to_string()
}

#[derive(Clone)]
pub struct Db {
    pool: AnyPool,
    backend: Backend,
    kingbase: bool,
    postgis: Arc<AtomicBool>,
}

impl Db {
    /// Open a database from a `TSP_DATABASE` value.
    ///
    /// PostgreSQL and MySQL/MariaDB URLs are used verbatim (except that
    /// `mariadb://` is normalized to the `mysql://` scheme sqlx expects). A
    /// filesystem path is converted to a SQLite URL with `mode=rwc` and its
    /// parent directory is created.
    pub async fn open(database: impl AsRef<str>) -> AppResult<Self> {
        let enabled = std::env::var("TSP_POSTGIS_ENABLED")
            .map(|value| !matches!(value.as_str(), "0" | "false" | "off"))
            .unwrap_or(true);
        Self::open_with_postgis(database, enabled).await
    }

    pub async fn open_with_postgis(
        database: impl AsRef<str>,
        postgis_enabled: bool,
    ) -> AppResult<Self> {
        let raw = database.as_ref();
        let kingbase = crate::config::is_kingbase_url(raw);
        let (backend, url) = if raw.starts_with("postgres://") || raw.starts_with("postgresql://") {
            (Backend::Postgres, raw.to_string())
        } else if kingbase {
            (Backend::Postgres, kingbase_url_from(raw))
        } else if raw.starts_with("mysql://") || raw.starts_with("mariadb://") {
            (Backend::Mysql, mysql_url_from(raw))
        } else if raw.starts_with("sqlite:") {
            (Backend::Sqlite, raw.to_string())
        } else {
            if let Some(parent) = Path::new(raw).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        tracing::error!(%error, "failed to create database directory");
                        AppError::internal("无法创建数据目录")
                    })?;
                }
            }
            (Backend::Sqlite, sqlite_url_from_path(raw))
        };
        Self::connect(url, backend, false, postgis_enabled, kingbase).await
    }

    /// Open a private in-memory SQLite database.
    ///
    /// The pool is limited to a single connection so every query in a test sees
    /// the same in-memory schema.
    pub async fn in_memory() -> AppResult<Self> {
        Self::connect(
            "sqlite::memory:".to_string(),
            Backend::Sqlite,
            true,
            false,
            false,
        )
        .await
    }

    async fn connect(
        url: String,
        backend: Backend,
        in_memory: bool,
        postgis_enabled: bool,
        kingbase: bool,
    ) -> AppResult<Self> {
        // Required before the first AnyPool/AnyConnection is created. The call
        // is idempotent (see sqlx::any::install_default_drivers).
        sqlx::any::install_default_drivers();

        let max_connections = if in_memory { 1 } else { 10 };
        let mut options = AnyPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(15))
            .after_connect(move |conn, _meta| {
                Box::pin(async move {
                    if backend == Backend::Sqlite {
                        sqlx::query("PRAGMA foreign_keys = ON")
                            .execute(&mut *conn)
                            .await?;
                        sqlx::query("PRAGMA busy_timeout = 5000")
                            .execute(&mut *conn)
                            .await?;
                        // WAL is meaningless for a private in-memory database and
                        // SQLite reports the current journal mode instead. Only
                        // enable it for file-backed databases.
                        if !in_memory {
                            sqlx::query("PRAGMA journal_mode = WAL")
                                .execute(&mut *conn)
                                .await?;
                        }
                    } else if backend == Backend::Mysql {
                        // The server default may be latin1; force utf8mb4 for
                        // the session so Chinese seed data and API payloads are
                        // stored correctly.
                        sqlx::query("SET NAMES utf8mb4 COLLATE utf8mb4_unicode_ci")
                            .execute(&mut *conn)
                            .await?;
                    }
                    Ok(())
                })
            });
        if in_memory {
            // Keep the single connection alive for the whole test/process so
            // the schema is not lost to idle eviction.
            options = options
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None);
        }

        let pool = options.connect(&url).await?;
        let db = Self {
            pool,
            backend,
            kingbase,
            postgis: Arc::new(AtomicBool::new(false)),
        };
        db.initialize().await?;
        if backend == Backend::Postgres {
            crate::db::indexes::ensure(&db).await?;
        }
        if backend == Backend::Postgres && postgis_enabled {
            match crate::db::postgis::enable(&db).await {
                Ok(true) => {
                    db.postgis.store(true, Ordering::Relaxed);
                    tracing::info!("PostGIS geometry columns and spatial indexes enabled");
                }
                Ok(false) => {
                    tracing::warn!("PostGIS is not available; falling back to lat/lon-only storage")
                }
                Err(error) => tracing::warn!(%error, "PostGIS setup failed; falling back"),
            }
        }
        Ok(db)
    }

    pub(crate) fn pool(&self) -> &AnyPool {
        &self.pool
    }

    /// True when the active backend is MySQL/MariaDB.
    pub(crate) fn is_mysql(&self) -> bool {
        self.backend == Backend::Mysql
    }

    pub(crate) fn is_sqlite(&self) -> bool {
        self.backend == Backend::Sqlite
    }

    pub fn postgis_enabled(&self) -> bool {
        self.postgis.load(Ordering::Relaxed)
    }

    pub fn is_kingbase(&self) -> bool {
        self.kingbase
    }

    pub fn engine_name(&self) -> &'static str {
        if self.kingbase {
            "kingbase"
        } else {
            match self.backend {
                Backend::Sqlite => "sqlite",
                Backend::Postgres => "postgres",
                Backend::Mysql => "mysql",
            }
        }
    }

    /// Return the canonical `?N` SQL unchanged.
    ///
    /// Backend translation happens in [`Db::query`]: PostgreSQL needs `$N`,
    /// MySQL needs unnamed markers plus reordered arguments, and SQLite accepts
    /// `?N` as-is.
    pub(crate) fn sql<'a>(&self, query: &'a str) -> Cow<'a, str> {
        if self.kingbase && query.contains("level") {
            // LEVEL is reserved in KingbaseES; PostgreSQL/SQLite/MySQL do not
            // need quoting here, so keep their SQL byte-for-byte unchanged.
            Cow::Owned(query.replace("level", "\"level\""))
        } else {
            Cow::Borrowed(query)
        }
    }

    /// Build a backend-portable query from canonical `?N` SQL.
    ///
    /// The wrapper defers placeholder translation and argument ordering until
    /// execution time (see [`DbQuery`]). `.bind(...)` values stay in numeric
    /// placeholder order, exactly as `sqlx::query(...).bind(...)` would expect.
    pub(crate) fn query<'a>(&self, query: &'a str) -> DbQuery<'a> {
        DbQuery::new(self.backend, query)
    }

    /// Cheap liveness probe used by `/api/health` and startup checks.
    pub async fn ping(&self) -> AppResult<()> {
        sqlx::query("SELECT 1").fetch_one(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
