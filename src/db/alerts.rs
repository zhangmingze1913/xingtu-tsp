use sqlx::Row;

use crate::{
    error::AppResult,
    models::{Alert, AlertListQuery, Page},
    util::{china_today_start_utc, now_iso, page_bounds},
};

use super::{get_i64, map_alert, Db, ALERT_COLS};

impl Db {
    pub async fn list_alerts(&self, query: AlertListQuery, org: &str) -> AppResult<Page<Alert>> {
        let (page, page_size, offset) = page_bounds(query.page, query.page_size);
        let status = query.status.unwrap_or_default();
        let level = query.level.unwrap_or_default();
        let vehicle_id = query.vehicle_id.unwrap_or_default();
        let condition = "WHERE (?1 = '' OR
                    (?1 = 'active' AND a.acknowledged = 0) OR
                    (?1 = 'acknowledged' AND a.acknowledged = 1))
                AND (?2 = '' OR a.level = ?2)
                AND (?3 = '' OR a.vehicle_id = ?3)
                AND (?4 = '*' OR v.org_name = ?4)";

        let count_query = format!(
            "SELECT COUNT(*) FROM alerts a JOIN vehicles v ON v.id = a.vehicle_id {condition}"
        );
        let count_sql = self.sql(&count_query);
        let count_row = self
            .query(&count_sql)
            .bind(status.as_str())
            .bind(level.as_str())
            .bind(vehicle_id.as_str())
            .bind(org)
            .fetch_one(self.pool())
            .await?;
        let total = get_i64(&count_row, 0)?;

        let query = format!(
            "SELECT {ALERT_COLS} FROM alerts a
             JOIN vehicles v ON v.id = a.vehicle_id
             {condition}
             ORDER BY a.ts DESC LIMIT ?5 OFFSET ?6"
        );
        let sql = self.sql(&query);
        let rows = self
            .query(&sql)
            .bind(status.as_str())
            .bind(level.as_str())
            .bind(vehicle_id.as_str())
            .bind(org)
            .bind(page_size)
            .bind(offset)
            .fetch_all(self.pool())
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push(map_alert(row)?);
        }
        Ok(Page {
            items,
            total,
            page,
            page_size,
        })
    }

    pub async fn get_alert(&self, id: &str) -> AppResult<Option<Alert>> {
        let query = format!(
            "SELECT {ALERT_COLS} FROM alerts a
             JOIN vehicles v ON v.id = a.vehicle_id WHERE a.id = ?1"
        );
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(map_alert(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn acknowledge_alert(&self, id: &str, user: &str) -> AppResult<Option<Alert>> {
        let sql = self.sql(
            "UPDATE alerts SET acknowledged = 1, ack_by = ?2, ack_at = ?3
             WHERE id = ?1 AND acknowledged = 0",
        );
        self.query(&sql)
            .bind(id)
            .bind(user)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        self.get_alert(id).await
    }

    pub async fn acknowledge_all_alerts(&self, user: &str) -> AppResult<i64> {
        let sql = self.sql(
            "UPDATE alerts SET acknowledged = 1, ack_by = ?1, ack_at = ?2
             WHERE acknowledged = 0",
        );
        let result = self
            .query(&sql)
            .bind(user)
            .bind(now_iso())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() as i64)
    }

    pub async fn alert_stats(&self) -> AppResult<(i64, i64, i64, i64)> {
        let today = china_today_start_utc();
        let active_sql = self.sql("SELECT COUNT(*) FROM alerts WHERE acknowledged = 0");
        let active_row = self.query(&active_sql).fetch_one(self.pool()).await?;
        let critical_sql =
            self.sql("SELECT COUNT(*) FROM alerts WHERE acknowledged = 0 AND level = 'critical'");
        let critical_row = self.query(&critical_sql).fetch_one(self.pool()).await?;
        let warning_sql =
            self.sql("SELECT COUNT(*) FROM alerts WHERE acknowledged = 0 AND level = 'warning'");
        let warning_row = self.query(&warning_sql).fetch_one(self.pool()).await?;
        let sql = self.sql("SELECT COUNT(*) FROM alerts WHERE ts >= ?1");
        let today_row = self.query(&sql).bind(&today).fetch_one(self.pool()).await?;
        Ok((
            get_i64(&active_row, 0)?,
            get_i64(&critical_row, 0)?,
            get_i64(&warning_row, 0)?,
            get_i64(&today_row, 0)?,
        ))
    }

    pub async fn recent_alerts(&self, limit: i64) -> AppResult<Vec<Alert>> {
        let limit = limit.clamp(1, 50);
        let query = format!(
            "SELECT {ALERT_COLS} FROM alerts a
             JOIN vehicles v ON v.id = a.vehicle_id
             ORDER BY a.ts DESC LIMIT ?1"
        );
        let sql = self.sql(&query);
        let rows = self.query(&sql).bind(limit).fetch_all(self.pool()).await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push(map_alert(row)?);
        }
        Ok(items)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_alert(
        &self,
        vehicle_id: &str,
        alert_type: &str,
        level: &str,
        title: &str,
        message: &str,
        lat: Option<f64>,
        lon: Option<f64>,
    ) -> AppResult<Option<Alert>> {
        let sql = self.sql("SELECT plate_no, model FROM vehicles WHERE id = ?1");
        let vehicle = self
            .query(&sql)
            .bind(vehicle_id)
            .fetch_optional(self.pool())
            .await?;
        let Some(vehicle) = vehicle else {
            return Ok(None);
        };
        let plate_no: String = vehicle.try_get(0)?;
        let model: String = vehicle.try_get(1)?;
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now_iso();
        let sql = self.sql(
            "INSERT INTO alerts
             (id, vehicle_id, alert_type, level, title, message, lat, lon, ts, acknowledged)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
        );
        self.query(&sql)
            .bind(&id)
            .bind(vehicle_id)
            .bind(alert_type)
            .bind(level)
            .bind(title)
            .bind(message)
            .bind(lat)
            .bind(lon)
            .bind(&ts)
            .execute(self.pool())
            .await?;
        Ok(Some(Alert {
            id,
            vehicle_id: vehicle_id.to_string(),
            alert_type: alert_type.to_string(),
            level: level.to_string(),
            title: title.to_string(),
            message: message.to_string(),
            lat,
            lon,
            ts,
            acknowledged: false,
            ack_by: None,
            ack_at: None,
            plate_no: Some(plate_no),
            model: Some(model),
        }))
    }
}
