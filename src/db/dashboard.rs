use std::collections::HashMap;

use crate::{
    error::AppResult,
    models::{AlertStats, CommandStats, DailyPoint, DashboardOverview, FleetStats, NameValue},
    util::{china_day_label, china_day_start_utc, china_today_start_utc, now_iso},
};

use sqlx::Row;

use super::{get_f64, get_i64, map_alert, Db, ALERT_COLS};

impl Db {
    /// `org` is `"*"` for administrators, or the user's organization name.
    pub async fn dashboard_overview(&self, org: &str) -> AppResult<DashboardOverview> {
        let today = china_today_start_utc();
        let days: Vec<(String, String, String)> = (0..7)
            .rev()
            .map(|days_ago| {
                (
                    china_day_start_utc(days_ago),
                    china_day_start_utc(days_ago - 1),
                    china_day_label(days_ago),
                )
            })
            .collect();
        let command_since = crate::util::iso_from_now_minutes(-60 * 24 * 7);
        let generated_at = now_iso();

        let fleet_future = async {
            let sql = self.sql(
                "SELECT COUNT(*),
                        COALESCE(AVG(s.soc), 0),
                        COALESCE(SUM(s.odometer_km), 0),
                        COUNT(t.vehicle_id),
                        COALESCE(SUM(CASE WHEN t.status = 'online' THEN 1 ELSE 0 END), 0)
                 FROM vehicles v
                 LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
                 LEFT JOIN terminals t ON t.vehicle_id = v.id
                 WHERE (?1 = '*' OR v.org_name = ?1)",
            );
            self.query(&sql).bind(org).fetch_one(self.pool()).await
        };
        let alert_command_future = async {
            let sql = self.sql(
                "SELECT
                    (SELECT COUNT(*) FROM alerts a
                     JOIN vehicles v ON v.id = a.vehicle_id
                     WHERE a.acknowledged = 0 AND (?1 = '*' OR v.org_name = ?1)),
                    (SELECT COUNT(*) FROM alerts a
                     JOIN vehicles v ON v.id = a.vehicle_id
                     WHERE a.acknowledged = 0 AND a.level = 'critical'
                       AND (?1 = '*' OR v.org_name = ?1)),
                    (SELECT COUNT(*) FROM alerts a
                     JOIN vehicles v ON v.id = a.vehicle_id
                     WHERE a.acknowledged = 0 AND a.level = 'warning'
                       AND (?1 = '*' OR v.org_name = ?1)),
                    (SELECT COUNT(*) FROM alerts a
                     JOIN vehicles v ON v.id = a.vehicle_id
                     WHERE a.ts >= ?2 AND (?1 = '*' OR v.org_name = ?1)),
                    (SELECT COUNT(*) FROM commands c
                     JOIN vehicles v ON v.id = c.vehicle_id
                     WHERE c.created_at >= ?3 AND (?1 = '*' OR v.org_name = ?1)),
                    (SELECT COUNT(*) FROM commands c
                     JOIN vehicles v ON v.id = c.vehicle_id
                     WHERE c.created_at >= ?3 AND c.status = 'success'
                       AND (?1 = '*' OR v.org_name = ?1))",
            );
            self.query(&sql)
                .bind(org)
                .bind(&today)
                .bind(&command_since)
                .fetch_one(self.pool())
                .await
        };
        let status_future = async {
            let sql = self.sql(
                "SELECT COALESCE(s.status, 'offline'), COUNT(*) FROM vehicles v
                 LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
                 WHERE (?1 = '*' OR v.org_name = ?1)
                 GROUP BY 1",
            );
            self.query(&sql).bind(org).fetch_all(self.pool()).await
        };
        let (fleet_row, stats_row, rows) =
            tokio::try_join!(fleet_future, alert_command_future, status_future)?;

        let total = get_i64(&fleet_row, 0)?;
        let avg_soc = get_f64(&fleet_row, 1)?;
        let total_mileage = get_f64(&fleet_row, 2)?;
        let terminal_total = get_i64(&fleet_row, 3)?;
        let terminal_online = get_i64(&fleet_row, 4)?;
        let active_alerts = get_i64(&stats_row, 0)?;
        let critical = get_i64(&stats_row, 1)?;
        let warning = get_i64(&stats_row, 2)?;
        let alerts_today = get_i64(&stats_row, 3)?;
        let commands_total = get_i64(&stats_row, 4)?;
        let commands_success = get_i64(&stats_row, 5)?;

        let mut status_counts: HashMap<String, i64> = HashMap::new();
        for row in &rows {
            let status: String = row.try_get(0)?;
            let count = get_i64(row, 1)?;
            status_counts.insert(status, count);
        }
        let get = |name: &str| *status_counts.get(name).unwrap_or(&0);
        let moving = get("moving");
        let idle = get("idle");
        let charging = get("charging");
        let online_extra = get("online");
        let offline = get("offline");
        let maintenance = get("maintenance");
        let online = moving + idle + charging + online_extra;

        let trend_futures = days.iter().map(|(start, end, label)| {
            let start = start.clone();
            let end = end.clone();
            let label = label.clone();
            let org = org.to_string();
            async move {
                let sql = self.sql(
                    "SELECT COALESCE(SUM(t.distance_km), 0) FROM trips t
                     JOIN vehicles v ON v.id = t.vehicle_id
                     WHERE t.start_ts >= ?1 AND t.start_ts < ?2
                       AND (?3 = '*' OR v.org_name = ?3)",
                );
                let mileage_row = self
                    .query(&sql)
                    .bind(&start)
                    .bind(&end)
                    .bind(&org)
                    .fetch_one(self.pool())
                    .await?;
                let mileage = get_f64(&mileage_row, 0)?;

                let sql = self.sql(
                    "SELECT COUNT(*) FROM alerts a
                     JOIN vehicles v ON v.id = a.vehicle_id
                     WHERE a.ts >= ?1 AND a.ts < ?2
                       AND (?3 = '*' OR v.org_name = ?3)",
                );
                let count_row = self
                    .query(&sql)
                    .bind(&start)
                    .bind(&end)
                    .bind(&org)
                    .fetch_one(self.pool())
                    .await?;
                let count = get_i64(&count_row, 0)?;
                Ok::<_, crate::error::AppError>((
                    DailyPoint {
                        label: label.clone(),
                        value: (mileage * 10.0).round() / 10.0,
                    },
                    DailyPoint {
                        label,
                        value: count as f64,
                    },
                ))
            }
        });
        let trend_pairs = futures_util::future::join_all(trend_futures).await;
        let mut mileage_7d = Vec::with_capacity(days.len());
        let mut alert_trend_7d = Vec::with_capacity(days.len());
        for pair in trend_pairs {
            let (mileage, alerts) = pair?;
            mileage_7d.push(mileage);
            alert_trend_7d.push(alerts);
        }

        let recent_alerts = {
            let query = format!(
                "SELECT {ALERT_COLS} FROM alerts a
                 JOIN vehicles v ON v.id = a.vehicle_id
                 WHERE (?1 = '*' OR v.org_name = ?1)
                 ORDER BY a.ts DESC LIMIT 6"
            );
            let sql = self.sql(&query);
            let rows = self.query(&sql).bind(org).fetch_all(self.pool()).await?;
            let mut items = Vec::with_capacity(rows.len());
            for row in &rows {
                items.push(map_alert(row)?);
            }
            items
        };

        let fleet = FleetStats {
            total,
            online,
            moving,
            idle,
            charging,
            offline,
            maintenance,
            online_rate: if total > 0 {
                ((online as f64 / total as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            },
            avg_soc: (avg_soc * 10.0).round() / 10.0,
            total_mileage_km: (total_mileage * 10.0).round() / 10.0,
            terminal_online_rate: if terminal_total > 0 {
                ((terminal_online as f64 / terminal_total as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            },
        };

        Ok(DashboardOverview {
            fleet,
            alerts: AlertStats {
                active: active_alerts,
                critical,
                warning,
                today: alerts_today,
            },
            mileage_7d,
            alert_trend_7d,
            status_distribution: vec![
                NameValue {
                    name: "moving".into(),
                    value: moving,
                },
                NameValue {
                    name: "idle".into(),
                    value: idle,
                },
                NameValue {
                    name: "charging".into(),
                    value: charging,
                },
                NameValue {
                    name: "online".into(),
                    value: online_extra,
                },
                NameValue {
                    name: "maintenance".into(),
                    value: maintenance,
                },
                NameValue {
                    name: "offline".into(),
                    value: offline,
                },
            ],
            recent_alerts,
            command: CommandStats {
                total_7d: commands_total,
                success_rate: if commands_total > 0 {
                    ((commands_success as f64 / commands_total as f64) * 1000.0).round() / 10.0
                } else {
                    0.0
                },
            },
            generated_at,
        })
    }
}
