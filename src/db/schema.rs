//! Dialect-specific bootstrap DDL.
//!
//! SQL lives in sibling `.sql` files so it can be reviewed with SQL tooling;
//! `include_str!` keeps it embedded in the binary with no runtime file lookup.

pub(crate) const SQLITE_SCHEMA: &str = include_str!("schema/sqlite.sql");
pub(crate) const POSTGRES_SCHEMA: &str = include_str!("schema/postgres.sql");
pub(crate) const MYSQL_SCHEMA: &str = include_str!("schema/mysql.sql");
