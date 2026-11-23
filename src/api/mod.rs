//! HTTP router and transport adapters.

pub mod alerts;
pub mod audit;
pub mod auth;
pub mod cluster;
pub mod commands;
pub mod dashboard;
pub mod gb32960;
pub mod geofences;
pub mod health;
pub mod integrations;
pub mod media;
pub mod security;
pub mod telemetry;
pub mod terminal_downlink;
pub mod terminals;
pub mod trips;
pub mod vehicles;
pub mod ws;

mod router;

pub(crate) use router::build_router;
