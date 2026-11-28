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

/// Opt-in Monoio runtime handle for Linux-only experiments.
#[cfg(feature = "monoio-runtime")]
pub use monoio as monoio_runtime;
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
mod vault_keys;
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
    services::{maintenance, simulator},
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
    maintenance::spawn_archive_cleanup();
    maintenance::spawn_archive_spool_replay();
    maintenance::spawn_tdengine_reconciler(state.clone());
    maintenance::spawn_timeseries_history_pruner(state.clone());
    maintenance::spawn_reconciliation_export_cleanup(state.clone());
    maintenance::spawn_reconciliation_export_recovery(state.clone());
    maintenance::spawn_vault_key_sync();

    maintenance::spawn_offline_sweeper(state.clone());
    maintenance::spawn_retention_sweeper(state.clone());
    maintenance::spawn_downlink_timeout_sweeper(state.clone());

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
