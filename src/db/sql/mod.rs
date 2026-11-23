//! SQLx portability layer: `?N` placeholders, URL normalization and schema
//! splitting for the supported engines.

mod placeholders;
mod query;
mod urls;

pub(crate) use placeholders::split_sql_statements;
#[cfg(test)]
pub(crate) use placeholders::{mysql_placeholders, rewrite_placeholders};
pub(crate) use query::DbQuery;
pub(crate) use urls::{mysql_url_from, sqlite_url_from_path};
