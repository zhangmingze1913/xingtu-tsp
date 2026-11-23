use sqlx::Row;

use crate::{
    error::{AppError, AppResult},
    models::{Terminal, TerminalRow},
    security::crypto::provider_from_env,
    util::now_iso,
};

use super::{get_i64, map_terminal, Db, TERMINAL_COLS};

const AUTH_CODE_VAULT_MARKER: &str = "__pii_vault__";

fn terminal_keyring() -> Option<std::sync::Arc<dyn crate::security::crypto::KeyProvider>> {
    provider_from_env()
}

impl Db {
    pub async fn list_terminals(&self) -> AppResult<Vec<TerminalRow>> {
        let query = format!(
            "SELECT {TERMINAL_COLS}, v.plate_no
             FROM terminals t
             LEFT JOIN vehicles v ON v.id = t.vehicle_id
             ORDER BY CASE t.status WHEN 'online' THEN 0 ELSE 1 END, t.imei"
        );
        let sql = self.sql(&query);
        let rows = self.query(&sql).fetch_all(self.pool()).await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push(TerminalRow {
                terminal: map_terminal(row)?,
                plate_no: row.try_get(13)?,
            });
        }
        Ok(items)
    }

    pub async fn get_terminal_by_imei(&self, imei: &str) -> AppResult<Option<Terminal>> {
        let query = format!("SELECT {TERMINAL_COLS} FROM terminals t WHERE t.imei = ?1");
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(imei)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_terminal(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn get_terminal_by_phone(&self, phone: &str) -> AppResult<Option<Terminal>> {
        let query = format!("SELECT {TERMINAL_COLS} FROM terminals t WHERE t.phone = ?1");
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(phone)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_terminal(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn get_terminal_by_vehicle(&self, vehicle_id: &str) -> AppResult<Option<Terminal>> {
        let query = format!("SELECT {TERMINAL_COLS} FROM terminals t WHERE t.vehicle_id = ?1");
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(vehicle_id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_terminal(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn get_terminal(&self, id: &str) -> AppResult<Option<Terminal>> {
        let query = format!("SELECT {TERMINAL_COLS} FROM terminals t WHERE t.id = ?1");
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_terminal(&row)?)),
            None => Ok(None),
        }
    }

    /// Registers a terminal from the JT/T 808 gateway. Existing auth codes are preserved
    /// so a re-registration does not invalidate an already provisioned device.
    pub async fn register_jt_terminal(
        &self,
        phone: &str,
        terminal_model: &str,
        auth_code: &str,
    ) -> AppResult<Terminal> {
        let imei = format!("86{phone}");
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now_iso();
        let sql = if self.is_mysql() {
            // MySQL has no ON CONFLICT; the unique imei key triggers the
            // duplicate-key branch. VALUES(col) exposes the row proposed by
            // the INSERT, while an unqualified column is the existing row.
            self.sql(
                "INSERT INTO terminals
                 (id, imei, phone, iccid, model, firmware, protocol, vehicle_id, status, auth_code,
                  last_heartbeat_at, created_at, bound_at)
                 VALUES (?1, ?2, ?3, '', ?4, '', 'JT/T 808-2019', NULL, 'online', ?5, ?6, ?6, NULL)
                 ON DUPLICATE KEY UPDATE
                    phone = VALUES(phone),
                    model = CASE WHEN VALUES(model) <> '' THEN VALUES(model) ELSE model END,
                    status = 'online',
                    last_heartbeat_at = VALUES(last_heartbeat_at),
                    auth_code = COALESCE(auth_code, VALUES(auth_code))",
            )
        } else {
            self.sql(
                "INSERT INTO terminals
                 (id, imei, phone, iccid, model, firmware, protocol, vehicle_id, status, auth_code,
                  last_heartbeat_at, created_at, bound_at)
                 VALUES (?1, ?2, ?3, '', ?4, '', 'JT/T 808-2019', NULL, 'online', ?5, ?6, ?6, NULL)
                 ON CONFLICT(imei) DO UPDATE SET
                    phone = excluded.phone,
                    model = CASE WHEN excluded.model <> '' THEN excluded.model ELSE terminals.model END,
                    status = 'online',
                    last_heartbeat_at = excluded.last_heartbeat_at,
                    auth_code = COALESCE(terminals.auth_code, excluded.auth_code)",
            )
        };
        let (stored_auth_code, vaulted) = match terminal_keyring() {
            Some(ring) => {
                let ciphertext = ring
                    .encrypt(auth_code)
                    .map_err(|error| crate::error::AppError::internal(error.to_string()))?;
                self.put_pii("terminal", phone, "auth_code", &ciphertext)
                    .await?;
                self.record_key_event(ring.provider_name(), "encrypt", true)
                    .await?;
                (AUTH_CODE_VAULT_MARKER.to_string(), true)
            }
            None => (auth_code.to_string(), false),
        };
        self.query(&sql)
            .bind(&id)
            .bind(&imei)
            .bind(phone)
            .bind(terminal_model)
            .bind(&stored_auth_code)
            .bind(&ts)
            .execute(self.pool())
            .await?;
        let query = format!("SELECT {TERMINAL_COLS} FROM terminals t WHERE t.imei = ?1");
        let select_sql = self.sql(&query);
        let row = self
            .query(&select_sql)
            .bind(&imei)
            .fetch_one(self.pool())
            .await?;
        let mut terminal = map_terminal(&row)?;
        if vaulted {
            terminal.auth_code = Some(auth_code.to_string());
        }
        Ok(terminal)
    }

    pub async fn verify_terminal_auth(&self, phone: &str, auth_code: &str) -> AppResult<bool> {
        let Some(terminal) = self.get_terminal_by_phone(phone).await? else {
            return Ok(false);
        };
        let matches = match terminal.auth_code.as_deref() {
            Some(AUTH_CODE_VAULT_MARKER) => match terminal_keyring() {
                Some(ring) => match self.get_pii("terminal", phone, "auth_code").await? {
                    Some(ciphertext) => {
                        ring.decrypt(&ciphertext).ok().as_deref() == Some(auth_code)
                    }
                    None => false,
                },
                None => false,
            },
            Some(code) => code == auth_code,
            None => false,
        };
        if matches {
            let ts = now_iso();
            let sql = self.sql(
                "UPDATE terminals SET status = 'online', last_heartbeat_at = ?2 WHERE phone = ?1",
            );
            self.query(&sql)
                .bind(phone)
                .bind(&ts)
                .execute(self.pool())
                .await?;
        }
        Ok(matches)
    }

    pub async fn heartbeat_terminal(&self, phone: &str) -> AppResult<()> {
        let ts = now_iso();
        let sql = self
            .sql("UPDATE terminals SET status = 'online', last_heartbeat_at = ?2 WHERE phone = ?1");
        self.query(&sql)
            .bind(phone)
            .bind(&ts)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn bind_terminal(
        &self,
        terminal_id: &str,
        vehicle_id: Option<String>,
    ) -> AppResult<Terminal> {
        let mut tx = self.pool().begin().await?;
        let query = format!("SELECT {TERMINAL_COLS} FROM terminals t WHERE t.id = ?1");
        let select_sql = self.sql(&query);
        let terminal = self
            .query(&select_sql)
            .bind(terminal_id)
            .fetch_optional(&mut tx)
            .await?;
        if terminal.is_none() {
            return Err(AppError::not_found("终端不存在"));
        }
        if let Some(vehicle_id) = vehicle_id {
            let exists_sql = self.sql("SELECT COUNT(*) FROM vehicles WHERE id = ?1");
            let exists_row = self
                .query(&exists_sql)
                .bind(&vehicle_id)
                .fetch_one(&mut tx)
                .await?;
            if get_i64(&exists_row, 0)? == 0 {
                return Err(AppError::not_found("车辆不存在"));
            }
            let occupied_sql =
                self.sql("SELECT imei FROM terminals WHERE vehicle_id = ?1 AND id <> ?2");
            let occupied = self
                .query(&occupied_sql)
                .bind(&vehicle_id)
                .bind(terminal_id)
                .fetch_optional(&mut tx)
                .await?;
            if let Some(other) = occupied {
                let other: String = other.try_get(0)?;
                return Err(AppError::conflict(format!(
                    "该车辆已绑定终端 {other}，请先解绑"
                )));
            }
            let update_sql = self.sql(
                "UPDATE terminals SET vehicle_id = ?2, bound_at = ?3, status = 'online'
                 WHERE id = ?1",
            );
            self.query(&update_sql)
                .bind(terminal_id)
                .bind(&vehicle_id)
                .bind(now_iso())
                .execute(&mut tx)
                .await?;
        } else {
            let update_sql =
                self.sql("UPDATE terminals SET vehicle_id = NULL, bound_at = NULL WHERE id = ?1");
            self.query(&update_sql)
                .bind(terminal_id)
                .execute(&mut tx)
                .await?;
        }
        let row = self
            .query(&select_sql)
            .bind(terminal_id)
            .fetch_one(&mut tx)
            .await?;
        let updated = map_terminal(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn create_and_bind_terminal(
        &self,
        vehicle_id: &str,
        imei: &str,
        phone: &str,
        iccid: &str,
        model: &str,
    ) -> AppResult<Terminal> {
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now_iso();
        let mut tx = self.pool().begin().await?;
        let exists_sql = self.sql("SELECT COUNT(*) FROM vehicles WHERE id = ?1");
        let exists_row = self
            .query(&exists_sql)
            .bind(vehicle_id)
            .fetch_one(&mut tx)
            .await?;
        if get_i64(&exists_row, 0)? == 0 {
            return Err(AppError::not_found("车辆不存在"));
        }
        let occupied_sql = self.sql("SELECT imei FROM terminals WHERE vehicle_id = ?1");
        let occupied = self
            .query(&occupied_sql)
            .bind(vehicle_id)
            .fetch_optional(&mut tx)
            .await?;
        if let Some(other) = occupied {
            let other: String = other.try_get(0)?;
            return Err(AppError::conflict(format!("该车辆已绑定终端 {other}")));
        }
        let insert_sql = self.sql(
            "INSERT INTO terminals
             (id, imei, phone, iccid, model, firmware, protocol, vehicle_id, status, auth_code,
              last_heartbeat_at, created_at, bound_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '1.4.2', 'JT/T 808-2019', ?6, 'online', NULL, ?7, ?7, ?7)",
        );
        self.query(&insert_sql)
            .bind(&id)
            .bind(imei)
            .bind(phone)
            .bind(iccid)
            .bind(model)
            .bind(vehicle_id)
            .bind(&ts)
            .execute(&mut tx)
            .await?;
        let query = format!("SELECT {TERMINAL_COLS} FROM terminals t WHERE t.id = ?1");
        let select_sql = self.sql(&query);
        let row = self.query(&select_sql).bind(&id).fetch_one(&mut tx).await?;
        let terminal = map_terminal(&row)?;
        tx.commit().await?;
        Ok(terminal)
    }

    pub async fn mark_terminal_offline(&self, imei: &str) -> AppResult<()> {
        let sql = self
            .sql("UPDATE terminals SET status = 'offline' WHERE imei = ?1 AND vehicle_id IS NULL");
        self.query(&sql).bind(imei).execute(self.pool()).await?;
        Ok(())
    }

    pub async fn terminal_stats(&self) -> AppResult<(i64, i64)> {
        let total_row = sqlx::query("SELECT COUNT(*) FROM terminals")
            .fetch_one(self.pool())
            .await?;
        let online_row = sqlx::query("SELECT COUNT(*) FROM terminals WHERE status = 'online'")
            .fetch_one(self.pool())
            .await?;
        Ok((get_i64(&total_row, 0)?, get_i64(&online_row, 0)?))
    }
}
