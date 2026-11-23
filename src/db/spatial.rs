//! Spatial vehicle queries. PostGIS is used when enabled; otherwise the same
//! API falls back to latitude/longitude filtering for SQLite/MySQL.

use sqlx::Row;

use crate::{error::AppResult, models::MapVehicle, util::haversine_m};

use super::{get_bool, get_f64, get_opt_f64, Db};

impl Db {
    pub async fn vehicles_nearby(
        &self,
        lat: f64,
        lon: f64,
        radius_m: f64,
        status: Option<String>,
        org: &str,
    ) -> AppResult<Vec<MapVehicle>> {
        if self.postgis_enabled() {
            let status = status.unwrap_or_default();
            let sql = self.sql(
                "SELECT v.id, v.plate_no, v.model, s.lat, s.lon, s.speed, s.heading,
                        s.status, s.soc, s.ignition, s.odometer_km
                 FROM vehicles v
                 JOIN vehicle_state s ON s.vehicle_id = v.id
                 WHERE s.geom IS NOT NULL
                   AND ST_DWithin(
                         s.geom::geography,
                         ST_SetSRID(ST_MakePoint(?1, ?2), 4326)::geography,
                         ?3)
                   AND (?4 = '' OR s.status = ?4)
                   AND (?5 = '*' OR v.org_name = ?5)
                 ORDER BY v.plate_no
                 LIMIT 500",
            );
            let rows = self
                .query(&sql)
                .bind(lon)
                .bind(lat)
                .bind(radius_m)
                .bind(status.as_str())
                .bind(org)
                .fetch_all(self.pool())
                .await?;
            return Ok(rows.iter().map(map_map_vehicle).collect::<Result<_, _>>()?);
        }

        let vehicles = self.map_vehicles(status, org).await?;
        Ok(vehicles
            .into_iter()
            .filter(|vehicle| {
                vehicle
                    .lat
                    .zip(vehicle.lon)
                    .map(|(vlat, vlon)| haversine_m(lat, lon, vlat, vlon) <= radius_m)
                    .unwrap_or(false)
            })
            .collect())
    }

    pub async fn vehicles_in_bbox(
        &self,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        status: Option<String>,
        org: &str,
    ) -> AppResult<Vec<MapVehicle>> {
        if self.postgis_enabled() {
            let status = status.unwrap_or_default();
            let sql = self.sql(
                "SELECT v.id, v.plate_no, v.model, s.lat, s.lon, s.speed, s.heading,
                        s.status, s.soc, s.ignition, s.odometer_km
                 FROM vehicles v
                 JOIN vehicle_state s ON s.vehicle_id = v.id
                 WHERE s.geom IS NOT NULL
                   AND ST_Intersects(
                         s.geom,
                         ST_MakeEnvelope(?1, ?2, ?3, ?4, 4326))
                   AND (?5 = '' OR s.status = ?5)
                   AND (?6 = '*' OR v.org_name = ?6)
                 ORDER BY v.plate_no
                 LIMIT 500",
            );
            let rows = self
                .query(&sql)
                .bind(min_lon)
                .bind(min_lat)
                .bind(max_lon)
                .bind(max_lat)
                .bind(status.as_str())
                .bind(org)
                .fetch_all(self.pool())
                .await?;
            return Ok(rows.iter().map(map_map_vehicle).collect::<Result<_, _>>()?);
        }

        let vehicles = self.map_vehicles(status, org).await?;
        Ok(vehicles
            .into_iter()
            .filter(|vehicle| {
                vehicle
                    .lat
                    .zip(vehicle.lon)
                    .map(|(lat, lon)| {
                        lat >= min_lat && lat <= max_lat && lon >= min_lon && lon <= max_lon
                    })
                    .unwrap_or(false)
            })
            .collect())
    }
}

fn map_map_vehicle(row: &sqlx::any::AnyRow) -> sqlx::Result<MapVehicle> {
    Ok(MapVehicle {
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
    })
}
