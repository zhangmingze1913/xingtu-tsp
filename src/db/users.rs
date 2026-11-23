use sqlx::Row;

use crate::{
    auth::verify_password,
    error::AppResult,
    models::{AuditLog, AuditQuery, Page, User},
    util::{now_iso, page_bounds},
};

use super::{get_i64, Db};

impl Db {
    pub async fn user_count(&self) -> AppResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) FROM users")
            .fetch_one(self.pool())
            .await?;
        Ok(get_i64(&row, 0)?)
    }

    /// Used only by production startup validation. Demo installations are
    /// allowed to keep the documented admin/admin123 account.
    pub async fn has_default_admin_password(&self) -> AppResult<bool> {
        let Some((_, hash)) = self.find_user_by_username("admin").await? else {
            return Ok(false);
        };
        Ok(verify_password("admin123", &hash))
    }

    pub async fn recent_login_failures(&self, username: &str, since: &str) -> AppResult<i64> {
        let sql = self.sql(
            "SELECT COUNT(*) FROM audit_logs
             WHERE action = 'auth.login_failed' AND username = ?1 AND created_at >= ?2",
        );
        let row = self
            .query(&sql)
            .bind(username)
            .bind(since)
            .fetch_one(self.pool())
            .await?;
        Ok(get_i64(&row, 0)?)
    }

    pub async fn find_user_by_username(&self, username: &str) -> AppResult<Option<(User, String)>> {
        let sql = self.sql(
            "SELECT id, username, display_name, role, org_name, created_at, password_hash
             FROM users WHERE username = ?1",
        );
        let row = self
            .query(&sql)
            .bind(username)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let user = User {
            id: row.try_get(0)?,
            username: row.try_get(1)?,
            display_name: row.try_get(2)?,
            role: row.try_get(3)?,
            org_name: row.try_get(4)?,
            created_at: row.try_get(5)?,
        };
        Ok(Some((user, row.try_get(6)?)))
    }

    pub async fn find_user_by_id(&self, id: &str) -> AppResult<Option<User>> {
        let sql = self.sql(
            "SELECT id, username, display_name, role, org_name, created_at
             FROM users WHERE id = ?1",
        );
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(User {
            id: row.try_get(0)?,
            username: row.try_get(1)?,
            display_name: row.try_get(2)?,
            role: row.try_get(3)?,
            org_name: row.try_get(4)?,
            created_at: row.try_get(5)?,
        }))
    }

    pub async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        display_name: &str,
        role: &str,
        org_name: &str,
    ) -> AppResult<User> {
        let user = User {
            id: uuid::Uuid::new_v4().to_string(),
            username: username.to_string(),
            display_name: display_name.to_string(),
            role: role.to_string(),
            org_name: org_name.to_string(),
            created_at: now_iso(),
        };
        let sql = self.sql(
            "INSERT INTO users
             (id, username, password_hash, display_name, role, org_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        );
        self.query(&sql)
            .bind(&user.id)
            .bind(&user.username)
            .bind(password_hash)
            .bind(&user.display_name)
            .bind(&user.role)
            .bind(&user.org_name)
            .bind(&user.created_at)
            .execute(self.pool())
            .await?;
        Ok(user)
    }

    pub async fn audit(
        &self,
        user_id: Option<&str>,
        username: Option<&str>,
        action: &str,
        target: &str,
        detail: &str,
    ) -> AppResult<()> {
        let sql = self.sql(
            "INSERT INTO audit_logs (user_id, username, action, target, detail, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        );
        self.query(&sql)
            .bind(user_id)
            .bind(username)
            .bind(action)
            .bind(target)
            .bind(detail)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn list_audit(&self, query: AuditQuery) -> AppResult<Page<AuditLog>> {
        let (page, page_size, offset) = page_bounds(query.page, query.page_size);
        let action = query.action.filter(|value| !value.trim().is_empty());
        let (total, rows) = if let Some(action) = &action {
            let count_sql = self.sql("SELECT COUNT(*) FROM audit_logs WHERE action = ?1");
            let total = self
                .query(&count_sql)
                .bind(action)
                .fetch_one(self.pool())
                .await?;
            let sql = self.sql(
                "SELECT id, user_id, username, action, target, detail, created_at
                 FROM audit_logs WHERE action = ?1
                 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
            );
            let rows = self
                .query(&sql)
                .bind(action)
                .bind(page_size)
                .bind(offset)
                .fetch_all(self.pool())
                .await?;
            (get_i64(&total, 0)?, rows)
        } else {
            let count_sql = self.sql("SELECT COUNT(*) FROM audit_logs");
            let total = self.query(&count_sql).fetch_one(self.pool()).await?;
            let sql = self.sql(
                "SELECT id, user_id, username, action, target, detail, created_at
                 FROM audit_logs ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
            );
            let rows = self
                .query(&sql)
                .bind(page_size)
                .bind(offset)
                .fetch_all(self.pool())
                .await?;
            (get_i64(&total, 0)?, rows)
        };
        let items = rows
            .iter()
            .map(|row| {
                Ok(AuditLog {
                    id: get_i64(row, 0)?,
                    user_id: row.try_get(1)?,
                    username: row.try_get(2)?,
                    action: row.try_get(3)?,
                    target: row.try_get(4)?,
                    detail: row.try_get(5)?,
                    created_at: row.try_get(6)?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(Page {
            items,
            total,
            page,
            page_size,
        })
    }

    pub async fn prune_audit_before(&self, cutoff: &str) -> AppResult<u64> {
        let sql = self.sql("DELETE FROM audit_logs WHERE created_at < ?1");
        Ok(self
            .query(&sql)
            .bind(cutoff)
            .execute(self.pool())
            .await?
            .rows_affected())
    }
}
