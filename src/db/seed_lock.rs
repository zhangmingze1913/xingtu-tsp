//! Cross-replica seed lock: PostgreSQL advisory lock, MySQL GET_LOCK,
//! SQLite single-process no-op.

use sqlx::{pool::PoolConnection, Any, Row};

use crate::error::{AppError, AppResult};

use super::{Backend, Db};

const SEED_LOCK_ID: i64 = 8_120_261_115;
const SEED_LOCK_NAME: &str = "xingtu_tsp_seed";

impl Db {
    /// Serialize first-boot demo seeding across API replicas.
    ///
    /// PostgreSQL uses a session advisory lock, MySQL uses `GET_LOCK()`, and
    /// SQLite (which has no cross-process lock here) is a no-op. The guard
    /// closes its dedicated pooled connection if it is dropped without an
    /// explicit release, so a lock cannot leak into the pool on early return.
    pub async fn acquire_seed_lock(&self) -> AppResult<SeedLock> {
        match self.backend {
            Backend::Sqlite => Ok(SeedLock::default()),
            Backend::Postgres => {
                let mut connection = self.pool.acquire().await?;
                if let Err(error) = sqlx::query("SELECT pg_advisory_lock($1)")
                    .bind(SEED_LOCK_ID)
                    .execute(&mut *connection)
                    .await
                {
                    if self.kingbase {
                        tracing::warn!(%error, "Kingbase seed advisory lock unavailable; seeding without cross-replica lock");
                        return Ok(SeedLock::default());
                    }
                    return Err(error.into());
                }
                Ok(SeedLock {
                    backend: Backend::Postgres,
                    connection: Some(connection),
                })
            }
            Backend::Mysql => {
                let mut connection = self.pool.acquire().await?;
                let row = sqlx::query("SELECT GET_LOCK(?, 30)")
                    .bind(SEED_LOCK_NAME)
                    .fetch_one(&mut *connection)
                    .await?;
                let acquired: Option<i64> = row.try_get(0)?;
                if acquired != Some(1) {
                    return Err(AppError::internal("无法获取 MySQL 种子数据锁"));
                }
                Ok(SeedLock {
                    backend: Backend::Mysql,
                    connection: Some(connection),
                })
            }
        }
    }
}

pub struct SeedLock {
    backend: Backend,
    connection: Option<PoolConnection<Any>>,
}

impl Default for SeedLock {
    fn default() -> Self {
        Self {
            backend: Backend::Sqlite,
            connection: None,
        }
    }
}

impl SeedLock {
    pub async fn release(mut self) {
        if let Some(mut connection) = self.connection.take() {
            match self.backend {
                Backend::Postgres => {
                    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
                        .bind(SEED_LOCK_ID)
                        .execute(&mut *connection)
                        .await;
                }
                Backend::Mysql => {
                    let _ = sqlx::query("SELECT RELEASE_LOCK(?)")
                        .bind(SEED_LOCK_NAME)
                        .execute(&mut *connection)
                        .await;
                }
                Backend::Sqlite => {}
            }
            self.connection = Some(connection);
        }
    }
}

impl Drop for SeedLock {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.detach();
        }
    }
}
