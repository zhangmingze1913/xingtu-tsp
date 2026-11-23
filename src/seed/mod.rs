//! Demo/bootstrap seeding.
//!
//! `admin.rs` handles the production first-admin path; `demo.rs` owns the
//! bundled dataset and its deterministic demo trips/telemetry.

mod admin;
mod builders;
mod demo;
mod fleet;
mod model;

pub use admin::ensure_admin;
pub use demo::seed_if_empty;
