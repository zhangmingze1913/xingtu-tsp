//! Demo fixture builders for geofences, alerts and commands.

use chrono::{DateTime, Duration, Utc};

use crate::models::{Command, Geofence, Vehicle, VehicleState};

use super::model::*;

pub(super) fn fences(demo: &DemoData, now: DateTime<Utc>) -> Vec<Geofence> {
    let fences: Vec<Geofence> = demo
        .geofences
        .iter()
        .map(|fence| Geofence {
            id: fence.id.clone(),
            name: fence.name.clone(),
            kind: fence.kind.clone(),
            center_lat: fence.center_lat,
            center_lon: fence.center_lon,
            radius_m: fence.radius_m,
            polygon: fence.polygon.clone(),
            color: fence.color.clone(),
            active: fence.active,
            created_at: iso(now - Duration::days(60)),
            org_name: String::new(),
            inside_count: 0,
        })
        .collect();
    fences
}

type AlertSeed = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<f64>,
    Option<f64>,
    String,
    bool,
    Option<String>,
    Option<String>,
);

pub(super) fn alerts(
    demo: &DemoData,
    states: &[VehicleState],
    vehicles: &[Vehicle],
    now: DateTime<Utc>,
) -> Vec<AlertSeed> {
    let mut alerts = Vec::new();
    for alert in &demo.alerts {
        let index = alert.vehicle_index % states.len();
        let state = &states[index];
        let ts = now - Duration::minutes(alert.minutes_ago);
        alerts.push((
            uuid::Uuid::new_v4().to_string(),
            vehicles[index].id.clone(),
            alert.alert_type.clone(),
            alert.level.clone(),
            alert.title.clone(),
            alert.message.clone(),
            state.lat,
            state.lon,
            iso(ts),
            alert.acknowledged,
            alert.ack_by.clone(),
            if alert.acknowledged {
                Some(iso(ts + Duration::minutes(8)))
            } else {
                None
            },
        ));
    }
    alerts
}

pub(super) fn commands(demo: &DemoData, vehicles: &[Vehicle], now: DateTime<Utc>) -> Vec<Command> {
    let mut commands = Vec::new();
    for command in &demo.commands {
        let index = command.vehicle_index % vehicles.len();
        let created = now - Duration::minutes(command.minutes_ago);
        commands.push(Command {
            id: uuid::Uuid::new_v4().to_string(),
            vehicle_id: vehicles[index].id.clone(),
            command: command.command.clone(),
            status: command.status.clone(),
            requested_by: command.requested_by.clone(),
            payload: None,
            result: Some(command.result.clone()),
            created_at: iso(created),
            sent_at: Some(iso(created + Duration::seconds(2))),
            completed_at: Some(iso(created + Duration::seconds(5))),
            plate_no: Some(vehicles[index].plate_no.clone()),
            model: Some(vehicles[index].model.clone()),
        });
    }
    commands
}
