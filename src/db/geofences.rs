use crate::{
    error::{AppError, AppResult},
    models::{CreateGeofenceRequest, Geofence, UpdateGeofenceRequest},
    util::{now_iso, validate_coord},
};

use super::{get_i64, map_geofence, Db, GEOFENCE_COLS};

impl Db {
    pub async fn list_geofences(&self, org: &str) -> AppResult<Vec<Geofence>> {
        let query = format!(
            "SELECT {GEOFENCE_COLS},
                (SELECT COUNT(*) FROM geofence_states gs
                 WHERE gs.geofence_id = g.id AND gs.inside = 1) AS inside_count
             FROM geofences g
             WHERE (?1 = '*' OR g.org_name = '' OR g.org_name = ?1)
             ORDER BY g.created_at DESC"
        );
        let sql = self.sql(&query);
        let rows = self.query(&sql).bind(org).fetch_all(self.pool()).await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut fence = map_geofence(row)?;
            fence.inside_count = get_i64(row, 11)?;
            items.push(fence);
        }
        Ok(items)
    }

    pub async fn get_geofence(&self, id: &str) -> AppResult<Option<Geofence>> {
        let query = format!(
            "SELECT {GEOFENCE_COLS},
                (SELECT COUNT(*) FROM geofence_states gs
                 WHERE gs.geofence_id = g.id AND gs.inside = 1)
             FROM geofences g WHERE g.id = ?1"
        );
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut fence = map_geofence(&row)?;
        fence.inside_count = get_i64(&row, 11)?;
        Ok(Some(fence))
    }

    pub async fn create_geofence(
        &self,
        req: CreateGeofenceRequest,
        org: &str,
    ) -> AppResult<Geofence> {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::bad_request("围栏名称不能为空"));
        }
        if !matches!(req.kind.as_str(), "circle" | "polygon") {
            return Err(AppError::bad_request("围栏类型仅支持 circle / polygon"));
        }

        let (center_lat, center_lon, polygon_json) = if req.kind == "polygon" {
            let polygon = req
                .polygon
                .as_ref()
                .filter(|polygon| polygon.len() >= 3)
                .ok_or_else(|| AppError::bad_request("多边形至少需要 3 个顶点"))?;
            for [lon, lat] in polygon {
                validate_coord(*lat, *lon).map_err(AppError::bad_request)?;
            }
            let lat = polygon.iter().map(|p| p[1]).sum::<f64>() / polygon.len() as f64;
            let lon = polygon.iter().map(|p| p[0]).sum::<f64>() / polygon.len() as f64;
            let json =
                serde_json::to_string(polygon).map_err(|_| AppError::internal("围栏序列化失败"))?;
            (lat, lon, Some(json))
        } else {
            let lat = req
                .center_lat
                .ok_or_else(|| AppError::bad_request("圆形围栏需要中心点纬度"))?;
            let lon = req
                .center_lon
                .ok_or_else(|| AppError::bad_request("圆形围栏需要中心点经度"))?;
            validate_coord(lat, lon).map_err(AppError::bad_request)?;
            (lat, lon, None)
        };

        if req.kind == "circle" && !(50.0..=50_000.0).contains(&req.radius_m) {
            return Err(AppError::bad_request("半径需在 50 至 50000 米之间"));
        }

        let fence = Geofence {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            kind: req.kind,
            center_lat,
            center_lon,
            radius_m: req.radius_m,
            polygon: req.polygon,
            color: if req.color.is_empty() {
                "#22d3ee".to_string()
            } else {
                req.color
            },
            active: req.active,
            created_at: now_iso(),
            org_name: org.to_string(),
            inside_count: 0,
        };
        let sql = self.sql(
            "INSERT INTO geofences
             (id, name, kind, center_lat, center_lon, radius_m, polygon, color, active, created_at, org_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        );
        self.query(&sql)
            .bind(&fence.id)
            .bind(&fence.name)
            .bind(&fence.kind)
            .bind(fence.center_lat)
            .bind(fence.center_lon)
            .bind(fence.radius_m)
            .bind(polygon_json)
            .bind(&fence.color)
            .bind(fence.active as i64)
            .bind(&fence.created_at)
            .bind(&fence.org_name)
            .execute(self.pool())
            .await?;
        Ok(fence)
    }

    pub async fn update_geofence(
        &self,
        id: &str,
        req: UpdateGeofenceRequest,
    ) -> AppResult<Option<Geofence>> {
        if let Some(radius) = req.radius_m {
            if !(50.0..=50_000.0).contains(&radius) {
                return Err(AppError::bad_request("半径需在 50 至 50000 米之间"));
            }
        }
        let sql = self.sql(
            "UPDATE geofences SET
                name = COALESCE(NULLIF(?2, ''), name),
                radius_m = COALESCE(?3, radius_m),
                color = COALESCE(?4, color),
                active = COALESCE(?5, active)
             WHERE id = ?1",
        );
        let result = self
            .query(&sql)
            .bind(id)
            .bind(req.name)
            .bind(req.radius_m)
            .bind(req.color)
            .bind(req.active.map(|value| value as i64))
            .execute(self.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_geofence(id).await
    }

    pub async fn delete_geofence(&self, id: &str) -> AppResult<bool> {
        let sql = self.sql("DELETE FROM geofences WHERE id = ?1");
        let result = self.query(&sql).bind(id).execute(self.pool()).await?;
        Ok(result.rows_affected() > 0)
    }
}
