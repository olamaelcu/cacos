//! Sea-orm ports of `rsky-pds/src/account_manager/helpers/*`.

pub mod account;

use sea_orm::{DatabaseBackend, Statement, Value};

/// Builds a raw SQLite statement. `?1..?N` placeholders bind positionally to
/// `values`, mirroring the reference's `params![]` ordering exactly.
pub(crate) fn sql(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
