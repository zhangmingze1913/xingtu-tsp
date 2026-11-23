//! Backend-aware query wrapper and executor target.

use std::borrow::Cow;

use sqlx::{
    any::{AnyArguments, AnyQueryResult, AnyRow},
    query::Query,
    Any, AnyPool, Arguments, Transaction,
};

use crate::db::Backend;

use super::placeholders::{mysql_placeholders, rewrite_placeholders};

/// Wrapper around [`sqlx::query`] that understands the canonical `?N`
/// placeholders used by the repositories.
///
/// * SQLite: `?N` is passed through unchanged and values are bound in numeric
///   order, which is SQLite's native behaviour.
/// * PostgreSQL: `?N` is rewritten to `$N`; PostgreSQL supports repeated and
///   out-of-order numbers, so values stay in numeric order.
/// * MySQL: only unnamed `?` markers are accepted. Every marker is rewritten in
///   occurrence order and the bound value for its number is duplicated, so
///   `?2 ... ?1 ... ?1` becomes `? ... ? ... ?` with arguments
///   `[value2, value1, value1]`.
///
/// Repository code always binds in numeric order (`?1`, then `?2`, ...), which
/// is what makes that expansion possible.
pub(crate) struct DbQuery<'q> {
    backend: Backend,
    sql: &'q str,
    // `AnyArguments` is the only argument container the sqlx facade exposes.
    // Its public `values` buffer is manipulated directly because the underlying
    // `AnyArgumentBuffer`/`AnyValueKind` types are not re-exported by sqlx.
    values: AnyArguments<'q>,
    error: Option<String>,
}

impl<'q> DbQuery<'q> {
    pub(crate) fn new(backend: Backend, sql: &'q str) -> Self {
        Self {
            backend,
            sql,
            values: AnyArguments::default(),
            error: None,
        }
    }

    /// Bind a value in numeric placeholder order.
    pub(crate) fn bind<T>(mut self, value: T) -> Self
    where
        T: 'q + sqlx::Encode<'q, Any> + sqlx::Type<Any>,
    {
        if self.error.is_some() {
            return self;
        }
        let mut scratch = AnyArguments::default();
        match scratch.add(value) {
            Ok(()) => self.values.values.0.append(&mut scratch.values.0),
            Err(error) => self.error = Some(error.to_string()),
        }
        self
    }

    pub(crate) async fn execute<'a, 't: 'a>(
        self,
        target: impl Into<DbTarget<'a, 't>>,
    ) -> sqlx::Result<AnyQueryResult> {
        let (sql, occurrences) = self.prepare_sql()?;
        let values = self.prepare_values(occurrences.as_deref())?;
        match target.into() {
            DbTarget::Pool(pool) => build_query(sql.as_ref(), values).execute(pool).await,
            DbTarget::Transaction(tx) => build_query(sql.as_ref(), values).execute(&mut **tx).await,
        }
    }

    pub(crate) async fn fetch_one<'a, 't: 'a>(
        self,
        target: impl Into<DbTarget<'a, 't>>,
    ) -> sqlx::Result<AnyRow> {
        let (sql, occurrences) = self.prepare_sql()?;
        let values = self.prepare_values(occurrences.as_deref())?;
        match target.into() {
            DbTarget::Pool(pool) => build_query(sql.as_ref(), values).fetch_one(pool).await,
            DbTarget::Transaction(tx) => {
                build_query(sql.as_ref(), values).fetch_one(&mut **tx).await
            }
        }
    }

    pub(crate) async fn fetch_optional<'a, 't: 'a>(
        self,
        target: impl Into<DbTarget<'a, 't>>,
    ) -> sqlx::Result<Option<AnyRow>> {
        let (sql, occurrences) = self.prepare_sql()?;
        let values = self.prepare_values(occurrences.as_deref())?;
        match target.into() {
            DbTarget::Pool(pool) => build_query(sql.as_ref(), values).fetch_optional(pool).await,
            DbTarget::Transaction(tx) => {
                build_query(sql.as_ref(), values)
                    .fetch_optional(&mut **tx)
                    .await
            }
        }
    }

    pub(crate) async fn fetch_all<'a, 't: 'a>(
        self,
        target: impl Into<DbTarget<'a, 't>>,
    ) -> sqlx::Result<Vec<AnyRow>> {
        let (sql, occurrences) = self.prepare_sql()?;
        let values = self.prepare_values(occurrences.as_deref())?;
        match target.into() {
            DbTarget::Pool(pool) => build_query(sql.as_ref(), values).fetch_all(pool).await,
            DbTarget::Transaction(tx) => {
                build_query(sql.as_ref(), values).fetch_all(&mut **tx).await
            }
        }
    }

    /// Translate the canonical SQL for the active backend.
    ///
    /// `Some(occurrences)` is returned only for MySQL: it lists the numeric
    /// parameter (zero based) for every `?` in the rewritten statement, in the
    /// order the markers appear.
    fn prepare_sql(&self) -> sqlx::Result<(Cow<'q, str>, Option<Vec<usize>>)> {
        if let Some(error) = &self.error {
            return Err(sqlx::Error::Encode(error.clone().into()));
        }
        match self.backend {
            Backend::Sqlite => Ok((Cow::Borrowed(self.sql), None)),
            Backend::Postgres => Ok((Cow::Owned(rewrite_placeholders(self.sql)), None)),
            Backend::Mysql => {
                let (sql, occurrences) = mysql_placeholders(self.sql);
                Ok((Cow::Owned(sql), Some(occurrences)))
            }
        }
    }

    /// Order the bound values for the active backend.
    ///
    /// SQLite and PostgreSQL bind in numeric order; MySQL must follow marker
    /// occurrence order and therefore duplicates repeated values.
    fn prepare_values(&self, occurrences: Option<&[usize]>) -> sqlx::Result<AnyArguments<'q>> {
        if let Some(error) = &self.error {
            return Err(sqlx::Error::Encode(error.clone().into()));
        }
        let mut values = AnyArguments::default();
        match occurrences {
            None => values.values.0.extend(self.values.values.0.iter().cloned()),
            Some(occurrences) => {
                for &index in occurrences {
                    let value = self.values.values.0.get(index).cloned().ok_or_else(|| {
                        sqlx::Error::Protocol(format!(
                            "SQL parameter ?{} has no bound value ({} bound)",
                            index + 1,
                            self.values.values.0.len()
                        ))
                    })?;
                    values.values.0.push(value);
                }
            }
        }
        Ok(values)
    }
}

/// Concrete executor accepted by [`DbQuery`].
///
/// A generic `sqlx::Executor` would tie the rewritten SQL's lifetime to the
/// caller's borrow, so the two concrete cases are reborrowed inside each method
/// instead. Transactions are used instead of `AnyConnection` because the sqlx
/// facade does not export the latter.
pub(crate) enum DbTarget<'a, 't: 'a> {
    Pool(&'a AnyPool),
    Transaction(&'a mut Transaction<'t, Any>),
}

impl<'a, 't: 'a> From<&'a AnyPool> for DbTarget<'a, 't> {
    fn from(pool: &'a AnyPool) -> Self {
        Self::Pool(pool)
    }
}

impl<'a, 't: 'a> From<&'a mut Transaction<'t, Any>> for DbTarget<'a, 't> {
    fn from(transaction: &'a mut Transaction<'t, Any>) -> Self {
        Self::Transaction(transaction)
    }
}

pub(crate) fn build_query<'s, 'v: 's>(
    sql: &'s str,
    values: AnyArguments<'v>,
) -> Query<'s, Any, AnyArguments<'s>> {
    let values: AnyArguments<'s> = values;
    sqlx::query_with(sql, values).persistent(true)
}
