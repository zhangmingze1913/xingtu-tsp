//! JT/T 808 location-derived alert rules (fence, overspeed, low SOC, harsh brake).

use std::collections::HashMap;

use sqlx::{Any, Row, Transaction};

use crate::{
    error::AppResult,
    models::{Alert, LocationSample},
    util::{haversine_m, iso_from_now_minutes, minutes_between, point_in_polygon},
};

use super::{get_f64, get_i64, Db};

struct Fence {
    id: String,
    name: String,
    kind: String,
    center_lat: f64,
    center_lon: f64,
    radius_m: f64,
    polygon_raw: Option<String>,
}

impl Db {
    pub(crate) async fn detect_alerts(
        &self,
        tx: &mut Transaction<'_, Any>,
        sample: &LocationSample,
        previous: &Option<super::telemetry::PreviousState>,
        plate_no: &str,
        model: &str,
        soc: f64,
    ) -> AppResult<Vec<Alert>> {
        let mut alerts = Vec::new();

        // Electronic fence transitions.
        let sql = self.sql(
            "SELECT id, name, kind, center_lat, center_lon, radius_m, polygon
             FROM geofences WHERE active = 1",
        );
        let rows = self.query(&sql).fetch_all(&mut *tx).await?;
        let mut fences = Vec::with_capacity(rows.len());
        for row in &rows {
            fences.push(Fence {
                id: row.try_get(0)?,
                name: row.try_get(1)?,
                kind: row.try_get(2)?,
                center_lat: get_f64(row, 3)?,
                center_lon: get_f64(row, 4)?,
                radius_m: get_f64(row, 5)?,
                polygon_raw: row.try_get(6)?,
            });
        }

        let prior_sql =
            self.sql("SELECT geofence_id, inside FROM geofence_states WHERE vehicle_id = ?1");
        let prior_rows = self
            .query(&prior_sql)
            .bind(sample.vehicle_id.as_str())
            .fetch_all(&mut *tx)
            .await?;
        let mut priors: HashMap<String, bool> = HashMap::with_capacity(prior_rows.len());
        for row in &prior_rows {
            priors.insert(row.try_get(0)?, get_i64(row, 1)? != 0);
        }
        let mut updates: Vec<(String, bool)> = Vec::with_capacity(fences.len());

        for fence in fences {
            let inside = if fence.kind == "polygon" {
                fence
                    .polygon_raw
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Vec<[f64; 2]>>(raw).ok())
                    .map(|polygon| point_in_polygon(sample.lat, sample.lon, &polygon))
                    .unwrap_or(false)
            } else {
                haversine_m(sample.lat, sample.lon, fence.center_lat, fence.center_lon)
                    <= fence.radius_m
            };

            let prior = priors.get(&fence.id).copied();
            updates.push((fence.id.clone(), inside));

            if let Some(prior_inside) = prior {
                if prior_inside != inside {
                    let (alert_type, level, title, action) = if inside {
                        ("geofence_enter", "info", "进入电子围栏", "进入")
                    } else {
                        ("geofence_exit", "warning", "驶出电子围栏", "驶出")
                    };
                    let message = format!(
                        "车辆 {plate_no} 于 {} {action}围栏「{}」",
                        sample.ts, fence.name
                    );
                    alerts.push(
                        self.insert_alert_tx(
                            &mut *tx,
                            &sample.vehicle_id,
                            plate_no,
                            model,
                            alert_type,
                            level,
                            title,
                            message,
                            sample.lat,
                            sample.lon,
                            &sample.ts,
                        )
                        .await?,
                    );
                }
            }
        }

        if !updates.is_empty() {
            let mut sql = String::from(
                "INSERT INTO geofence_states (vehicle_id, geofence_id, inside, updated_at) VALUES ",
            );
            for (index, _) in updates.iter().enumerate() {
                if index > 0 {
                    sql.push(',');
                }
                let n = index * 4;
                sql.push_str(&format!("(?{},?{},?{},?{})", n + 1, n + 2, n + 3, n + 4));
            }
            if self.is_mysql() {
                sql.push_str(
                    " ON DUPLICATE KEY UPDATE inside = VALUES(inside), updated_at = VALUES(updated_at)",
                );
            } else {
                sql.push_str(
                    " ON CONFLICT(vehicle_id, geofence_id) DO UPDATE SET inside = excluded.inside, updated_at = excluded.updated_at",
                );
            }
            let mut query = self.query(&sql);
            for (geofence_id, inside) in &updates {
                query = query
                    .bind(sample.vehicle_id.as_str())
                    .bind(geofence_id.as_str())
                    .bind(*inside as i64)
                    .bind(sample.ts.as_str());
            }
            query.execute(&mut *tx).await?;
        }

        // Overspeed: debounce five minutes per vehicle.
        if sample.speed > 120.0
            && !self
                .has_recent_alert_tx(
                    &mut *tx,
                    &sample.vehicle_id,
                    "overspeed",
                    &iso_from_now_minutes(-5),
                )
                .await?
        {
            let level = if sample.speed > 140.0 {
                "critical"
            } else {
                "warning"
            };
            alerts.push(
                self.insert_alert_tx(
                    &mut *tx,
                    &sample.vehicle_id,
                    plate_no,
                    model,
                    "overspeed",
                    level,
                    "超速告警",
                    format!(
                        "车辆 {plate_no} 当前车速 {:.0} km/h，超过 120 km/h 安全阈值",
                        sample.speed
                    ),
                    sample.lat,
                    sample.lon,
                    &sample.ts,
                )
                .await?,
            );
        }

        // Low state of charge: debounce thirty minutes.
        if sample.ignition
            && soc <= 15.0
            && !self
                .has_recent_alert_tx(
                    &mut *tx,
                    &sample.vehicle_id,
                    "low_soc",
                    &iso_from_now_minutes(-30),
                )
                .await?
        {
            let level = if soc < 8.0 { "critical" } else { "warning" };
            alerts.push(
                self.insert_alert_tx(
                    &mut *tx,
                    &sample.vehicle_id,
                    plate_no,
                    model,
                    "low_soc",
                    level,
                    "低电量预警",
                    format!("车辆 {plate_no} 剩余电量 {:.0}%，请及时规划补能", soc),
                    sample.lat,
                    sample.lon,
                    &sample.ts,
                )
                .await?,
            );
        }

        // Harsh braking inferred from a rapid speed drop within ten seconds.
        // Simulator samples intentionally skip this rule so synthetic overspeed
        // recovery does not flood the operator with duplicate events.
        if sample.source != "simulator" {
            if let Some(previous) = &previous {
                let gap_minutes = minutes_between(&previous.updated_at, &sample.ts);
                let drop = previous.speed - sample.speed;
                if (0.0..=10.0 / 60.0).contains(&gap_minutes)
                    && drop >= 20.0
                    && !self
                        .has_recent_alert_tx(
                            &mut *tx,
                            &sample.vehicle_id,
                            "harsh_brake",
                            &iso_from_now_minutes(-2),
                        )
                        .await?
                {
                    alerts.push(
                        self.insert_alert_tx(
                            &mut *tx,
                            &sample.vehicle_id,
                            plate_no,
                            model,
                            "harsh_brake",
                            "warning",
                            "急减速告警",
                            format!(
                                "车辆 {plate_no} 在 {:.1} 秒内减速 {:.0} km/h，疑似急刹",
                                gap_minutes * 60.0,
                                drop
                            ),
                            sample.lat,
                            sample.lon,
                            &sample.ts,
                        )
                        .await?,
                    );
                }
            }
        }
        Ok(alerts)
    }
}
