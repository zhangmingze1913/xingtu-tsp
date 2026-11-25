//! Encrypted PII vault: only ciphertext is stored here.

use sqlx::Row;

use crate::{
    db::Db,
    error::{AppError, AppResult},
    util::now_iso,
};

impl Db {
    /// Audit rows are lossy-on-crash by design: relaxing synchronous_commit
    /// keeps them off the read-path fsync latency. Business writes keep the
    /// server default.
    async fn relax_audit_commit(&self, tx: &mut sqlx::Transaction<'_, sqlx::Any>) -> AppResult<()> {
        if self.backend == crate::db::Backend::Postgres {
            if let Err(error) = sqlx::query("SET LOCAL synchronous_commit = off")
                .execute(&mut **tx)
                .await
            {
                if !self.is_kingbase() {
                    return Err(error.into());
                }
                tracing::warn!(%error, "Kingbase synchronous_commit setting unavailable");
            }
        }
        Ok(())
    }

    pub async fn put_pii(
        &self,
        owner_type: &str,
        owner_id: &str,
        field: &str,
        ciphertext: &str,
    ) -> AppResult<()> {
        self.query("DELETE FROM pii_vault WHERE owner_type = ?1 AND owner_id = ?2 AND field = ?3")
            .bind(owner_type)
            .bind(owner_id)
            .bind(field)
            .execute(self.pool())
            .await?;
        self.query(
            "INSERT INTO pii_vault (id, owner_type, owner_id, field, ciphertext, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(owner_type)
        .bind(owner_id)
        .bind(field)
        .bind(ciphertext)
        .bind(now_iso())
        .execute(self.pool())
        .await
        .map_err(AppError::from)?;
        Ok(())
    }

    pub async fn get_pii(
        &self,
        owner_type: &str,
        owner_id: &str,
        field: &str,
    ) -> AppResult<Option<String>> {
        let row = self
            .query(
                "SELECT ciphertext FROM pii_vault
                 WHERE owner_type = ?1 AND owner_id = ?2 AND field = ?3",
            )
            .bind(owner_type)
            .bind(owner_id)
            .bind(field)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|row| row.get::<String, _>(0)))
    }

    /// Read several fields for several owners in one query.
    pub async fn get_pii_batch(
        &self,
        owner_type: &str,
        owner_ids: &[String],
        fields: &[&str],
    ) -> AppResult<std::collections::HashMap<(String, String), String>> {
        if owner_ids.is_empty() || fields.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let field_marks: Vec<String> = (0..fields.len()).map(|i| format!("?{}", i + 2)).collect();
        let id_start = fields.len() + 2;
        let id_marks: Vec<String> = (0..owner_ids.len())
            .map(|i| format!("?{}", id_start + i))
            .collect();
        let sql = format!(
            "SELECT owner_id, field, ciphertext FROM pii_vault
             WHERE owner_type = ?1 AND field IN ({}) AND owner_id IN ({})",
            field_marks.join(", "),
            id_marks.join(", ")
        );
        let mut query = self.query(&sql).bind(owner_type);
        for field in fields {
            query = query.bind(*field);
        }
        for id in owner_ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(self.pool()).await?;
        let mut out = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            out.insert(
                (row.get::<String, _>(0), row.get::<String, _>(1)),
                row.get::<String, _>(2),
            );
        }
        Ok(out)
    }

    pub async fn delete_pii(
        &self,
        owner_type: &str,
        owner_id: &str,
        field: &str,
    ) -> AppResult<u64> {
        let result = self
            .query("DELETE FROM pii_vault WHERE owner_type = ?1 AND owner_id = ?2 AND field = ?3")
            .bind(owner_type)
            .bind(owner_id)
            .bind(field)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }
}

impl Db {
    pub async fn record_key_event(
        &self,
        provider: &str,
        action: &str,
        success: bool,
    ) -> AppResult<()> {
        self.query(
            "INSERT INTO key_audit (id, provider, action, success, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(provider)
        .bind(action)
        .bind(if success { 1i64 } else { 0i64 })
        .bind(now_iso())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Record several audit events with multi-row INSERTs (chunked to stay
    /// under SQLite's bind-variable limit).
    pub async fn record_key_events(
        &self,
        provider: &str,
        action: &str,
        successes: &[bool],
    ) -> AppResult<()> {
        if successes.is_empty() {
            return Ok(());
        }
        const CHUNK: usize = 100;
        let mut tx = self.pool().begin().await?;
        self.relax_audit_commit(&mut tx).await?;
        for chunk in successes.chunks(CHUNK) {
            let mut sql = String::from(
                "INSERT INTO key_audit (id, provider, action, success, created_at) VALUES ",
            );
            let mut bind = 0usize;
            for index in 0..chunk.len() {
                if index > 0 {
                    sql.push(',');
                }
                sql.push_str(&format!(
                    "(?{}, ?{}, ?{}, ?{}, ?{})",
                    bind + 1,
                    bind + 2,
                    bind + 3,
                    bind + 4,
                    bind + 5
                ));
                bind += 5;
            }
            let mut query = self.query(&sql);
            for success in chunk {
                query = query
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(provider)
                    .bind(action)
                    .bind(if *success { 1i64 } else { 0i64 })
                    .bind(now_iso());
            }
            query.execute(&mut tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Record a batch outcome with at most two audit rows (success/failure).
    pub async fn record_key_event_outcome(
        &self,
        provider: &str,
        action: &str,
        successes: &[bool],
    ) -> AppResult<()> {
        if successes.is_empty() {
            return Ok(());
        }
        let any_ok = successes.iter().any(|ok| *ok);
        let any_fail = successes.iter().any(|ok| !*ok);
        let mut tx = self.pool().begin().await?;
        self.relax_audit_commit(&mut tx).await?;
        for success in [any_ok, any_fail] {
            let skip = if success { !any_ok } else { !any_fail };
            if skip {
                continue;
            }
            self.query(
                "INSERT INTO key_audit (id, provider, action, success, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(provider)
            .bind(action)
            .bind(if success { 1i64 } else { 0i64 })
            .bind(now_iso())
            .execute(&mut tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn count_key_events(&self, action: &str) -> AppResult<i64> {
        let row = self
            .query("SELECT COUNT(*) FROM key_audit WHERE action = ?1")
            .bind(action)
            .fetch_one(self.pool())
            .await?;
        Ok(sqlx::Row::get::<i64, _>(&row, 0))
    }
}

impl Db {
    pub async fn purge_pii_before(&self, cutoff: &str) -> AppResult<u64> {
        let result = self
            .query("DELETE FROM pii_vault WHERE updated_at < ?1")
            .bind(cutoff)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn purge_key_audit_before(&self, cutoff: &str) -> AppResult<u64> {
        let result = self
            .query("DELETE FROM key_audit WHERE created_at < ?1")
            .bind(cutoff)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }
}
