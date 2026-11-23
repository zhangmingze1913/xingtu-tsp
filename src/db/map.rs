//! Map projection query kept separate from vehicle CRUD.

use sqlx::Row;

use crate::{error::AppResult, models::MapVehicle};

use super::{get_bool, get_f64, get_opt_f64, Db};

impl Db {
    pub async fn map_vehicles(
        &self,
        status: Option<String>,
        org: &str,
    ) -> AppResult<Vec<MapVehicle>> {
        let status = status.unwrap_or_default();
        let sql = self.sql(
            "SELECT v.id, v.plate_no, v.model, s.lat, s.lon, s.speed, s.heading,
                    s.status, s.soc, s.ignition, s.odometer_km
             FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
             WHERE (?1 = '' OR s.status = ?1)
               AND (?2 = '*' OR v.org_name = ?2)
             ORDER BY v.plate_no
             LIMIT 500",
        );
        let rows = self
            .query(&sql)
            .bind(status.as_str())
            .bind(org)
            .fetch_all(self.pool())
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push(MapVehicle {
                id: row.try_get(0)?,
                plate_no: row.try_get(1)?,
                model: row.try_get(2)?,
                lat: get_opt_f64(row, 3)?,
                lon: get_opt_f64(row, 4)?,
                speed: get_f64(row, 5)?,
                heading: get_f64(row, 6)?,
                status: row.try_get(7)?,
                soc: get_f64(row, 8)?,
                ignition: get_bool(row, 9)?,
                odometer_km: get_f64(row, 10)?,
            });
        }
        Ok(items)
    }
}
