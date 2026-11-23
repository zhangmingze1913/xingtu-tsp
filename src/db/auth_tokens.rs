//! Opaque refresh-token persistence and revocation.
//!
//! Only the SHA-256 hash is stored; the raw token is returned to the client
//! once and never logged.

use sqlx::Row;

use crate::{error::AppResult, util::now_iso};

use super::Db;

pub(crate) struct RefreshRecord {
    pub id: String,
    pub user_id: String,
}

impl Db {
    pub(crate) async fn store_refresh_token(
        &self,
        id: &str,
        user_id: &str,
        token_hash: &str,
        expires_at: &str,
    ) -> AppResult<()> {
        let sql = self.sql(
            "INSERT INTO refresh_tokens
             (id, user_id, token_hash, expires_at, revoked_at, created_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
        );
        self.query(&sql)
            .bind(id)
            .bind(user_id)
            .bind(token_hash)
            .bind(expires_at)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub(crate) async fn find_active_refresh(
        &self,
        token_hash: &str,
    ) -> AppResult<Option<RefreshRecord>> {
        let sql = self.sql(
            "SELECT id, user_id FROM refresh_tokens
             WHERE token_hash = ?1 AND revoked_at IS NULL AND expires_at > ?2",
        );
        let row = self
            .query(&sql)
            .bind(token_hash)
            .bind(now_iso())
            .fetch_optional(self.pool())
            .await?;
        row.map(|row| {
            Ok(RefreshRecord {
                id: row.try_get(0)?,
                user_id: row.try_get(1)?,
            })
        })
        .transpose()
    }

    pub(crate) async fn revoke_refresh(&self, id: &str) -> AppResult<()> {
        let sql = self.sql(
            "UPDATE refresh_tokens SET revoked_at = ?2
             WHERE id = ?1 AND revoked_at IS NULL",
        );
        self.query(&sql)
            .bind(id)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub(crate) async fn revoke_refresh_by_hash(&self, token_hash: &str) -> AppResult<()> {
        let sql = self.sql(
            "UPDATE refresh_tokens SET revoked_at = ?2
             WHERE token_hash = ?1 AND revoked_at IS NULL",
        );
        self.query(&sql)
            .bind(token_hash)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub(crate) async fn prune_refresh_tokens(&self, before: &str) -> AppResult<u64> {
        let sql = self.sql(
            "DELETE FROM refresh_tokens
             WHERE expires_at < ?1 OR (revoked_at IS NOT NULL AND revoked_at < ?1)",
        );
        Ok(self
            .query(&sql)
            .bind(before)
            .execute(self.pool())
            .await?
            .rows_affected())
    }
}
