//! Last reported 0x0104 parameter list and 0x0107 attributes per terminal.

use sqlx::Row;

use crate::{error::AppResult, util::now_iso};

use super::Db;

impl Db {
    pub(crate) async fn save_terminal_parameters(
        &self,
        terminal_id: &str,
        parameters_json: &str,
    ) -> AppResult<()> {
        let sql = if self.is_mysql() {
            self.sql(
                "INSERT INTO terminal_profiles (terminal_id, parameters_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON DUPLICATE KEY UPDATE parameters_json = VALUES(parameters_json),
                                         updated_at = VALUES(updated_at)",
            )
        } else {
            self.sql(
                "INSERT INTO terminal_profiles (terminal_id, parameters_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(terminal_id) DO UPDATE SET
                    parameters_json = excluded.parameters_json,
                    updated_at = excluded.updated_at",
            )
        };
        self.query(&sql)
            .bind(terminal_id)
            .bind(parameters_json)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub(crate) async fn save_terminal_attributes(
        &self,
        terminal_id: &str,
        attributes_json: &str,
    ) -> AppResult<()> {
        let sql = if self.is_mysql() {
            self.sql(
                "INSERT INTO terminal_profiles (terminal_id, attributes_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON DUPLICATE KEY UPDATE attributes_json = VALUES(attributes_json),
                                         updated_at = VALUES(updated_at)",
            )
        } else {
            self.sql(
                "INSERT INTO terminal_profiles (terminal_id, attributes_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(terminal_id) DO UPDATE SET
                    attributes_json = excluded.attributes_json,
                    updated_at = excluded.updated_at",
            )
        };
        self.query(&sql)
            .bind(terminal_id)
            .bind(attributes_json)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub(crate) async fn terminal_profile(
        &self,
        terminal_id: &str,
    ) -> AppResult<Option<(Option<String>, Option<String>, String)>> {
        let sql = self.sql(
            "SELECT parameters_json, attributes_json, updated_at
             FROM terminal_profiles WHERE terminal_id = ?1",
        );
        let row = self
            .query(&sql)
            .bind(terminal_id)
            .fetch_optional(self.pool())
            .await?;
        row.map(|row| Ok((row.try_get(0)?, row.try_get(1)?, row.try_get(2)?)))
            .transpose()
    }

    pub(crate) async fn save_terminal_registration(
        &self,
        terminal_id: &str,
        registration_json: &str,
    ) -> AppResult<()> {
        let sql = if self.is_mysql() {
            self.sql(
                "INSERT INTO terminal_profiles (terminal_id, registration_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON DUPLICATE KEY UPDATE registration_json = VALUES(registration_json),
                                         updated_at = VALUES(updated_at)",
            )
        } else {
            self.sql(
                "INSERT INTO terminal_profiles (terminal_id, registration_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(terminal_id) DO UPDATE SET
                    registration_json = excluded.registration_json,
                    updated_at = excluded.updated_at",
            )
        };
        self.query(&sql)
            .bind(terminal_id)
            .bind(registration_json)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub(crate) async fn terminal_registration(
        &self,
        terminal_id: &str,
    ) -> AppResult<Option<String>> {
        let sql =
            self.sql("SELECT registration_json FROM terminal_profiles WHERE terminal_id = ?1");
        let row = self
            .query(&sql)
            .bind(terminal_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|row| row.try_get(0)).transpose()?)
    }

    pub(crate) async fn save_terminal_parameter_version(
        &self,
        terminal_id: &str,
        parameters_json: &str,
    ) -> AppResult<i64> {
        let row = self
            .query(
                "SELECT COALESCE(MAX(version), 0) + 1
                 FROM terminal_parameter_versions WHERE terminal_id = ?1",
            )
            .bind(terminal_id)
            .fetch_one(self.pool())
            .await?;
        let version: i64 = row.try_get(0)?;
        self.query(
            "INSERT INTO terminal_parameter_versions
             (id, terminal_id, version, parameters_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(terminal_id)
        .bind(version)
        .bind(parameters_json)
        .bind(now_iso())
        .execute(self.pool())
        .await?;
        Ok(version)
    }

    pub(crate) async fn terminal_parameter_versions(
        &self,
        terminal_id: &str,
        limit: i64,
    ) -> AppResult<Vec<(i64, String, String)>> {
        let rows = self
            .query(
                "SELECT version, parameters_json, created_at
                 FROM terminal_parameter_versions
                 WHERE terminal_id = ?1
                 ORDER BY version DESC LIMIT ?2",
            )
            .bind(terminal_id)
            .bind(limit)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get(0)?, row.try_get(1)?, row.try_get(2)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parameter_versions_are_ordered() {
        let db = Db::in_memory().await.expect("db");
        crate::seed::seed_if_empty(&db).await.expect("seed");
        let terminals = db.list_terminals().await.expect("terminals");
        let phone = terminals.first().expect("terminal").terminal.phone.clone();
        let terminal = db
            .get_terminal_by_phone(&phone)
            .await
            .expect("lookup")
            .expect("terminal");
        assert_eq!(
            db.save_terminal_parameter_version(&terminal.id, "{\"a\":1}")
                .await
                .expect("v1"),
            1
        );
        assert_eq!(
            db.save_terminal_parameter_version(&terminal.id, "{\"a\":2}")
                .await
                .expect("v2"),
            2
        );
        let history = db
            .terminal_parameter_versions(&terminal.id, 10)
            .await
            .expect("history");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].0, 2);
        assert_eq!(history[1].0, 1);
    }
}
