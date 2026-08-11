//! Account helpers, each one operating on the shared sea-orm `DatabaseConnection`
//! via raw `Statement::from_sql_and_values` SQL.

pub mod account;
pub mod admin_tokens;
pub mod auth;
pub mod email_token;
pub mod init_required_keys;
pub mod invite;
pub mod password;
pub mod repo;
pub mod secret_provider;
pub mod secrets;

use sea_orm::{DatabaseBackend, Statement, Value};

/// Builds a raw SQLite statement. `?1..?N` placeholders bind positionally to
/// `values`.
pub(crate) fn sql(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
