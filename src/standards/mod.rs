//! Standards integration boundaries.
//!
//! These are **transport/codec skeletons** for the platform-exchange standards,
//! not full conformance implementations. They give the TSP a stable place to
//! add message-body mapping and conformance tests without pretending that the
//! current demo is already certified.

pub mod catalog;
pub mod gb19056;
pub mod gb32960;
pub mod gb32960_gateway;
pub mod gb32960_ingest;
pub mod jt1078;
pub mod jt809;
