use sqlx::Row;

use crate::{
    error::{AppError, AppResult},
    models::{Command, CommandListQuery, Page},
    util::{now_iso, page_bounds},
};

use super::{get_i64, map_command, Db, COMMAND_COLS};

pub const ALLOWED_COMMANDS: &[&str] = &[
    "lock",
    "unlock",
    "start",
    "stop",
    "flash",
    "honk",
    "ac_on",
    "ac_off",
    "charge_start",
    "charge_stop",
    "window_close",
    "find",
];

impl Db {
    pub async fn create_command(
        &self,
        vehicle_id: &str,
        command: &str,
        payload: Option<serde_json::Value>,
        requested_by: &str,
    ) -> AppResult<Command> {
        if !ALLOWED_COMMANDS.contains(&command) {
            return Err(AppError::bad_request(format!("不支持的指令：{command}")));
        }
        let payload = payload.map(|value| value.to_string());
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now_iso();

        let sql = self.sql(
            "SELECT v.plate_no, v.model, s.status FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id WHERE v.id = ?1",
        );
        let vehicle = self
            .query(&sql)
            .bind(vehicle_id)
            .fetch_optional(self.pool())
            .await?;
        let Some(vehicle) = vehicle else {
            return Err(AppError::not_found("车辆不存在"));
        };
        let plate_no: String = vehicle.try_get(0)?;
        let model: String = vehicle.try_get(1)?;
        let status: Option<String> = vehicle.try_get(2)?;
        let status = status.unwrap_or_else(|| "offline".to_string());
        if matches!(status.as_str(), "offline" | "maintenance") {
            return Err(AppError::conflict("车辆当前离线或维护中，无法下发远程指令"));
        }
        let sql = self.sql(
            "INSERT INTO commands
             (id, vehicle_id, command, status, requested_by, payload, result,
              created_at, sent_at, completed_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, NULL, ?6, NULL, NULL)",
        );
        self.query(&sql)
            .bind(&id)
            .bind(vehicle_id)
            .bind(command)
            .bind(requested_by)
            .bind(&payload)
            .bind(&ts)
            .execute(self.pool())
            .await?;
        Ok(Command {
            id,
            vehicle_id: vehicle_id.to_string(),
            command: command.to_string(),
            status: "pending".to_string(),
            requested_by: requested_by.to_string(),
            payload,
            result: None,
            created_at: ts,
            sent_at: None,
            completed_at: None,
            plate_no: Some(plate_no),
            model: Some(model),
        })
    }

    pub async fn set_command_status(
        &self,
        id: &str,
        status: &str,
        result: Option<String>,
    ) -> AppResult<Option<Command>> {
        let query = format!(
            "SELECT {COMMAND_COLS} FROM commands c
             JOIN vehicles v ON v.id = c.vehicle_id WHERE c.id = ?1"
        );
        let select_sql = self.sql(&query);
        let ts = now_iso();
        let mut tx = self.pool().begin().await?;
        let existing = self
            .query(&select_sql)
            .bind(id)
            .fetch_optional(&mut tx)
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        let existing = map_command(&existing)?;
        let sent_at = if status == "sent" {
            Some(ts.clone())
        } else {
            existing.sent_at.clone()
        };
        let completed_at = if matches!(status, "success" | "failed" | "cancelled") {
            Some(ts.clone())
        } else {
            existing.completed_at.clone()
        };
        let sql = self.sql(
            "UPDATE commands SET status = ?2, result = COALESCE(?3, result),
                sent_at = ?4, completed_at = ?5 WHERE id = ?1",
        );
        self.query(&sql)
            .bind(id)
            .bind(status)
            .bind(result)
            .bind(sent_at)
            .bind(completed_at)
            .execute(&mut tx)
            .await?;
        let row = self.query(&select_sql).bind(id).fetch_one(&mut tx).await?;
        let updated = map_command(&row)?;
        tx.commit().await?;
        Ok(Some(updated))
    }

    pub async fn get_command(&self, id: &str) -> AppResult<Option<Command>> {
        let query = format!(
            "SELECT {COMMAND_COLS} FROM commands c
             JOIN vehicles v ON v.id = c.vehicle_id WHERE c.id = ?1"
        );
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_command(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn list_commands(
        &self,
        query: CommandListQuery,
        org: &str,
    ) -> AppResult<Page<Command>> {
        let (page, page_size, offset) = page_bounds(query.page, query.page_size);
        let status = query.status.unwrap_or_default();
        let vehicle_id = query.vehicle_id.unwrap_or_default();
        let condition = "WHERE (?1 = '' OR c.status = ?1)
                AND (?2 = '' OR c.vehicle_id = ?2)
                AND (?3 = '*' OR v.org_name = ?3)";

        let count_query = format!(
            "SELECT COUNT(*) FROM commands c
             JOIN vehicles v ON v.id = c.vehicle_id {condition}"
        );
        let count_sql = self.sql(&count_query);
        let count_row = self
            .query(&count_sql)
            .bind(status.as_str())
            .bind(vehicle_id.as_str())
            .bind(org)
            .fetch_one(self.pool())
            .await?;
        let total = get_i64(&count_row, 0)?;

        let query = format!(
            "SELECT {COMMAND_COLS} FROM commands c
             JOIN vehicles v ON v.id = c.vehicle_id
             {condition}
             ORDER BY c.created_at DESC LIMIT ?4 OFFSET ?5"
        );
        let sql = self.sql(&query);
        let rows = self
            .query(&sql)
            .bind(status.as_str())
            .bind(vehicle_id.as_str())
            .bind(org)
            .bind(page_size)
            .bind(offset)
            .fetch_all(self.pool())
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push(map_command(row)?);
        }
        Ok(Page {
            items,
            total,
            page,
            page_size,
        })
    }

    pub async fn command_stats(&self) -> AppResult<(i64, i64)> {
        let since = crate::util::iso_from_now_minutes(-60 * 24 * 7);
        let sql = self.sql("SELECT COUNT(*) FROM commands WHERE created_at >= ?1");
        let total_row = self.query(&sql).bind(&since).fetch_one(self.pool()).await?;
        let sql =
            self.sql("SELECT COUNT(*) FROM commands WHERE created_at >= ?1 AND status = 'success'");
        let success_row = self.query(&sql).bind(&since).fetch_one(self.pool()).await?;
        Ok((get_i64(&total_row, 0)?, get_i64(&success_row, 0)?))
    }

    pub async fn seed_command(&self, command: &Command) -> AppResult<()> {
        let sql = self.sql(
            "INSERT INTO commands
             (id, vehicle_id, command, status, requested_by, payload, result,
              created_at, sent_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        );
        self.query(&sql)
            .bind(&command.id)
            .bind(&command.vehicle_id)
            .bind(&command.command)
            .bind(&command.status)
            .bind(&command.requested_by)
            .bind(&command.payload)
            .bind(&command.result)
            .bind(&command.created_at)
            .bind(&command.sent_at)
            .bind(&command.completed_at)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}
