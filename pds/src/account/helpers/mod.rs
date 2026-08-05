//! Account helpers, each one operating on the shared sea-orm `DatabaseConnection`
//! via raw `Statement::from_sql_and_values` SQL.

pub mod account;
pub mod password;

use sea_orm::{DatabaseBackend, Statement, Value};

/// Builds a raw SQLite statement. `?1..?N` placeholders bind positionally to
/// `values`.
pub(crate) fn sql(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
