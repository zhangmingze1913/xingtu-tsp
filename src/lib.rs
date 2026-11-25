#![recursion_limit = "256"]
#![forbid(unsafe_code)]
//! Xingtu TSP: a Rust reference implementation of a vehicle telematics
//! service platform.
//!
//! The crate is intentionally layered:
//! - `api` / `state`: transport and application wiring
//! - `db` / `models`: persistence and domain records
//! - `jt808` / `standards`: protocol gateways
//! - `cluster` / `raft*`: coordination and control plane
//! - `services`: background workers
//!
//! The public surface is intentionally narrow: core modules (`config`, `db`,
//! `auth`, `seed`, `state`, `standards`) plus selected 808 codec/peer helpers;
//! transport and adapter modules stay crate-private

mod api;
pub mod auth;
mod auth_keys;
mod cluster;
pub mod config;
pub mod db;
pub mod error;
mod export_crypto;
mod iggy_bus;
pub mod jt808;
pub mod media;
mod metrics;
pub mod models;
mod raft;
mod raft_log;
mod raft_state;
mod realtime;
pub mod security;
pub mod seed;
mod services;
pub mod standards;
pub mod state;
mod tdengine;
mod tdengine_spool;
mod timeseries;
mod util;
mod walrus_bus;

// Selected public re-exports for the bundled simulator and library consumers.
pub use jt808::{build_frame, parse_frame, MSG_LOCATION_REPORT, MSG_TERMINAL_AUTH};
pub use raft::parse_peers;

mod bootstrap;

use std::time::Duration;

use axum::Router;
use base64::Engine as _;
use tokio::net::TcpListener;

use crate::{
    cluster::Cluster,
    config::Config,
    error::{AppError, AppResult},
    services::simulator,
    state::AppState,
};

pub fn build_app(state: AppState) -> Router {
    api::build_router(state)
}

/// Boot dependencies, start background workers and serve HTTP.
pub async fn run(config: Config) -> AppResult<()> {
    let state = bootstrap::bootstrap(&config).await?;

    if let Some(archive) = crate::media::archive::global().cloned() {
        tokio::spawn(async move {
            if let Err(error) = archive.ensure_bucket().await {
                tracing::warn!(%error, "RustFS media bucket is not available");
            }
        });
    }
    spawn_archive_cleanup();
    spawn_archive_spool_replay();
    spawn_tdengine_reconciler(state.clone());
    spawn_timeseries_history_pruner(state.clone());
    spawn_reconciliation_export_cleanup(state.clone());
    spawn_reconciliation_export_recovery(state.clone());

    spawn_offline_sweeper(state.clone());
    spawn_retention_sweeper(state.clone());
    spawn_downlink_timeout_sweeper(state.clone());

    if let Ok(addr) = std::env::var("TSP_MEDIA_UDP_ADDR") {
        tokio::spawn(async move {
            if let Err(error) = crate::media::udp::serve(&addr).await {
                tracing::error!(%error, "JT/T 1078 media UDP listener stopped");
            }
        });
    }

    if let Ok(addr) = std::env::var("TSP_GB32960_TCP_ADDR") {
        let db = state.db.clone();
        let tls = match (
            std::env::var("TSP_GB32960_TLS_CERT"),
            std::env::var("TSP_GB32960_TLS_KEY"),
        ) {
            (Ok(cert), Ok(key)) => Some((cert, key)),
            _ => None,
        };
        let client_ca = std::env::var("TSP_GB32960_CLIENT_CA").ok();
        tokio::spawn(async move {
            let result = match tls {
                Some((cert, key)) => {
                    crate::standards::gb32960_gateway::serve_tls(
                        &addr,
                        db,
                        std::path::Path::new(&cert),
                        std::path::Path::new(&key),
                        client_ca.as_deref().map(std::path::Path::new),
                    )
                    .await
                }
                None => crate::standards::gb32960_gateway::serve(&addr, db).await,
            };
            if let Err(error) = result {
                tracing::error!(%error, "GB/T 32960 TCP listener stopped");
            }
        });
    }

    if config.demo_mode {
        simulator::start(state.clone(), state.cluster.leader_flag());
        tracing::info!("embedded fleet simulator enabled (leader-elected in cluster mode)");
    }

    let gateway_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = jt808::serve(gateway_state).await {
            tracing::error!(%error, "JT/T 808 gateway stopped");
        }
    });

    let app = build_app(state);
    let listener = TcpListener::bind(&config.bind).await.map_err(|error| {
        tracing::error!(%error, addr = %config.bind, "HTTP bind failed");
        AppError::internal(format!("无法监听 {}", config.bind))
    })?;
    tracing::info!(
        url = %format!("http://{}", config.bind),
        cluster = config.cluster_mode,
        "Xingtu TSP console ready"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| {
            tracing::error!(%error, "HTTP server failed");
            AppError::internal("HTTP 服务异常退出")
        })?;
    tracing::info!("server stopped gracefully");
    Ok(())
}

pub(crate) fn spawn_gateway_command_subscriber(cluster: &Cluster) {
    cluster.spawn_gateway_subscriber(|payload| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            tracing::debug!("invalid gateway command payload");
            return;
        };
        let Some(phone) = value.get("phone").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(serial) = value.get("serial").and_then(|v| v.as_u64()) else {
            return;
        };
        let Some(command_id) = value.get("command_id").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(frame) = value
            .get("frame")
            .and_then(|v| v.as_str())
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        else {
            return;
        };
        if !command_id.is_empty() {
            jt808::register_pending_command(phone, serial as u16, command_id);
        }
        if !jt808::deliver_local_frame(phone, frame) {
            tracing::warn!(
                phone,
                command_id,
                "remote gateway command could not be delivered"
            );
        }
    });
}

pub(crate) fn spawn_jt808_kick_subscriber(cluster: &Cluster) {
    let self_node = cluster.node_id().to_string();
    cluster.spawn_jt808_kick_subscriber(move |payload| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            return;
        };
        let Some(phone) = value.get("phone").and_then(|v| v.as_str()) else {
            return;
        };
        let owner = value.get("node_id").and_then(|v| v.as_str()).unwrap_or("");
        if owner != self_node {
            tracing::info!(
                phone,
                owner,
                "JT808 session moved; cancelling local connection"
            );
            jt808::cancel_terminal_session(phone);
        }
    });
}

fn spawn_reconciliation_export_recovery(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            match state.db.expired_export_job_ids(&now).await {
                Ok(ids) => {
                    for id in ids {
                        crate::api::integrations::recover_export_job(state.clone(), id).await;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "export recovery scan failed");
                }
            }
        }
    });
}

fn spawn_reconciliation_export_cleanup(state: AppState) {
    let hours = std::env::var("TSP_RECONCILE_EXPORT_RETENTION_HOURS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(24);
    if hours <= 0 {
        return;
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            if let Ok(pruned) = state.db.prune_export_download_tokens(&now).await {
                if pruned > 0 {
                    tracing::info!(pruned, "pruned expired export download tokens");
                }
            }
            let cutoff = (chrono::Utc::now() - chrono::Duration::hours(hours))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let ids = match state
                .db
                .prune_reconciliation_export_jobs_before(&cutoff)
                .await
            {
                Ok(ids) => ids,
                Err(error) => {
                    tracing::warn!(%error, "reconciliation export cleanup failed");
                    continue;
                }
            };
            if ids.is_empty() {
                continue;
            }
            let dir = std::env::var("TSP_RECONCILE_EXPORT_DIR")
                .unwrap_or_else(|_| "data/reconcile-exports".to_string());
            for id in &ids {
                for suffix in [".csv", ".csv.tmp", ".csv.enc"] {
                    let _ = tokio::fs::remove_file(format!("{dir}/{id}{suffix}")).await;
                }
            }
            if let Ok(pruned) = state.db.prune_export_alert_state(&cutoff).await {
                if pruned > 0 {
                    tracing::info!(pruned, "pruned export alert state");
                }
            }
            tracing::info!(jobs = ids.len(), hours, "pruned reconciliation export jobs");
        }
    });
}

fn spawn_timeseries_history_pruner(state: AppState) {
    let days = std::env::var("TSP_TIMESERIES_RECONCILE_HISTORY_DAYS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(90);
    if days <= 0 {
        return;
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(6 * 3600));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let cutoff = (chrono::Utc::now() - chrono::Duration::days(days))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            match state
                .db
                .prune_timeseries_reconciliations_before(&cutoff)
                .await
            {
                Ok(0) => {}
                Ok(deleted) => {
                    crate::tdengine::HISTORY_PRUNED
                        .fetch_add(deleted, std::sync::atomic::Ordering::Relaxed);
                    tracing::info!(deleted, days, "pruned reconciliation history");
                }
                Err(error) => tracing::warn!(%error, "reconciliation history prune failed"),
            }
        }
    });
}

fn spawn_tdengine_reconciler(state: AppState) {
    if state.tdengine.is_none()
        || state.config.timeseries_backend == crate::config::TimeSeriesBackend::Postgres
    {
        return;
    }
    let enabled = std::env::var("TSP_TIMESERIES_RECONCILE_ENABLED")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "off"))
        .unwrap_or(true);
    if !enabled {
        return;
    }
    let interval_secs = std::env::var("TSP_TIMESERIES_RECONCILE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(60, 86_400))
        .unwrap_or(300);
    let window_minutes = std::env::var("TSP_TIMESERIES_RECONCILE_WINDOW_MINUTES")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.clamp(1, 1440))
        .unwrap_or(15);
    let max_vehicles = std::env::var("TSP_TIMESERIES_RECONCILE_MAX_VEHICLES")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.clamp(1, 1000))
        .unwrap_or(100);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if state.cluster.enabled() && !state.cluster.is_leader() {
                continue;
            }
            let to = chrono::Utc::now();
            let from = to - chrono::Duration::minutes(window_minutes);
            let from = from.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let to = to.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let ids = match state
                .db
                .recent_telemetry_vehicle_ids(&from, max_vehicles)
                .await
            {
                Ok(ids) => ids,
                Err(error) => {
                    tracing::warn!(%error, "TDengine reconciliation vehicle query failed");
                    continue;
                }
            };
            let Some(tdengine) = state.tdengine.as_ref() else {
                continue;
            };
            let alert_mode = std::env::var("TSP_RECONCILE_ALERT_MODE")
                .unwrap_or_else(|_| "per_vehicle".to_string());
            let mut mismatches = 0;
            let mut mismatch_ids = Vec::new();
            for id in &ids {
                let pg = match state.db.telemetry_range_stats(id, &from, &to).await {
                    Ok(stats) => stats,
                    Err(error) => {
                        tracing::warn!(%error, vehicle = %id, "PG reconciliation query failed");
                        continue;
                    }
                };
                let td = match tdengine.range_stats(id, &from, &to).await {
                    Ok(stats) => stats,
                    Err(error) => {
                        tracing::warn!(%error, vehicle = %id, "TDengine reconciliation query failed");
                        continue;
                    }
                };
                crate::tdengine::RECONCILE_CHECKS
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let delta = pg.count - td.count;
                if delta != 0 {
                    mismatches += 1;
                    crate::tdengine::RECONCILE_MISMATCHES
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::warn!(
                        vehicle = %id,
                        pg = pg.count,
                        tdengine = td.count,
                        delta,
                        "TDengine/PG reconciliation mismatch"
                    );
                    let org_name = state
                        .db
                        .vehicle_org_name(id)
                        .await
                        .unwrap_or_default()
                        .unwrap_or_default();
                    if let Err(error) = state
                        .db
                        .record_timeseries_reconciliation(
                            id, &org_name, &from, &to, pg.count, td.count, delta,
                        )
                        .await
                    {
                        tracing::warn!(%error, vehicle = %id, "reconciliation history write failed");
                    }
                    if alert_mode.eq_ignore_ascii_case("summary") {
                        mismatch_ids.push(id.clone());
                    } else {
                        crate::tdengine::notify_reconcile_mismatch(
                            id, &from, &to, pg.count, td.count, delta,
                        )
                        .await;
                    }
                }
            }
            if alert_mode.eq_ignore_ascii_case("summary") && !mismatch_ids.is_empty() {
                crate::tdengine::notify_reconcile_summary(&mismatch_ids, &from, &to).await;
            }
            if mismatches > 0 {
                tracing::warn!(
                    vehicles = ids.len(),
                    mismatches,
                    "TDengine reconciliation completed with mismatches"
                );
            }
        }
    });
}

fn spawn_archive_spool_replay() {
    let Some(archive) = crate::media::archive::global().cloned() else {
        return;
    };
    let Some(spool) = crate::media::archive_spool::global().cloned() else {
        return;
    };
    let interval = Duration::from_secs(crate::media::archive_spool::replay_interval_seconds());
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let replayed = crate::media::archive::replay_spool(&archive, &spool).await;
            if replayed > 0 {
                tracing::info!(replayed, "archive spool replay completed");
            }
        }
    });
}

fn spawn_archive_cleanup() {
    let days = crate::media::archive::retention_days();
    if days == 0 {
        return;
    }
    let Some(archive) = crate::media::archive::global().cloned() else {
        return;
    };
    let interval = Duration::from_secs(crate::media::archive::cleanup_interval_seconds());
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match archive.cleanup_expired(days as i64 * 86_400).await {
                Ok(0) => {}
                Ok(deleted) => {
                    tracing::info!(deleted, retention_days = days, "RustFS archive cleanup")
                }
                Err(error) => tracing::warn!(%error, "RustFS archive cleanup failed"),
            }
        }
    });
}

fn spawn_downlink_timeout_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut retried = std::collections::HashSet::new();
        loop {
            interval.tick().await;
            let expired = jt808::expire_stale_downlinks(120);
            if expired.is_empty() {
                continue;
            }
            tracing::warn!(count = expired.len(), "JT808 downlinks timed out");
            for status in expired {
                if status.msg_id != jt808::MSG_SET_PARAMETERS
                    || !retried.insert(status.phone.clone())
                {
                    continue;
                }
                match crate::api::terminal_downlink::retry_latest_parameters(&state, &status.phone)
                    .await
                {
                    Ok((serial, version)) => tracing::warn!(
                        phone = %status.phone,
                        serial,
                        version,
                        "JT808 parameter downlink retried after timeout"
                    ),
                    Err(error) => tracing::warn!(
                        %error,
                        phone = %status.phone,
                        "JT808 parameter auto-retry failed"
                    ),
                }
            }
        }
    });
}

fn spawn_offline_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !state.cluster.is_leader() {
                continue;
            }
            let cutoff = crate::util::iso_from_now_minutes(-2);
            match state
                .db
                .mark_stale_offline(&cutoff, &crate::util::now_iso())
                .await
            {
                Ok((0, 0)) => {}
                Ok((terminals, vehicles)) => {
                    state
                        .metrics
                        .offline_sweeps
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::info!(
                        terminals,
                        vehicles,
                        "stale terminals/vehicles marked offline"
                    );
                }
                Err(error) => tracing::warn!(%error, "offline sweep failed"),
            }
        }
    });
}

fn spawn_retention_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(6 * 3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !state.cluster.is_leader() {
                continue;
            }
            let telemetry_cutoff = crate::util::iso_from_now_minutes(-7 * 24 * 60);
            let audit_cutoff = crate::util::iso_from_now_minutes(-180 * 24 * 60);
            match state.db.prune_telemetry_before(&telemetry_cutoff).await {
                Ok(rows) if rows > 0 => tracing::info!(rows, "pruned old telemetry"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "telemetry retention failed"),
            }
            match state.db.prune_audit_before(&audit_cutoff).await {
                Ok(rows) if rows > 0 => tracing::info!(rows, "pruned old audit logs"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "audit retention failed"),
            }
            let policy = crate::security::retention::RetentionPolicy::default();
            match state.db.purge_pii_before(&policy.pii_cutoff()).await {
                Ok(rows) if rows > 0 => tracing::info!(rows, "purged expired PII vault rows"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "PII retention failed"),
            }
            match state
                .db
                .purge_key_audit_before(&policy.key_audit_cutoff())
                .await
            {
                Ok(rows) if rows > 0 => tracing::info!(rows, "purged expired key audit rows"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "key audit retention failed"),
            }
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        else {
            return;
        };
        signal.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
}
