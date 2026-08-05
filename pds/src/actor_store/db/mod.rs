//! Per-actor SQLite database access.
//!
//! The per-actor database is a plain `sea_orm::DatabaseConnection`
//! (sqlx-sqlite pool); queries go through sea-orm entities where declarative
//! and `Statement::from_sql_and_values` otherwise. Opening runs the actor
//! migrations (a no-op on an already-migrated file) via
//! `DatabaseKind::Actor.open`.

use sea_orm::DatabaseConnection;

/// Re-export `DatabaseKind` so callers can do
/// `crate::actor_store::db::DatabaseKind::Actor.open(path)`.
pub use crate::db::DatabaseKind;

/// The per-actor database handle. A named alias over
/// `sea_orm::DatabaseConnection` for readability.
pub type ActorDb = DatabaseConnection;

// Re-export the migration entities this module's consumers need. Entity
// Models replace the plan's private row structs (`RepoBlock`, `Record`,
// `Backlink`); every query that previously mapped a row struct now uses
// `repo_block::Model` / `record::Model` / `backlink::Model` directly.
pub use crate::db::entities::{backlink, record, repo_block, repo_root};

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    #[tokio::test]
    async fn migrates_actor_db_schema_with_all_tables() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = DatabaseKind::Actor
            .open(dir.path().join("store.sqlite"))
            .await
            .unwrap();
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
            vec![],
        );
        let rows = db.query_all_raw(stmt).await.unwrap();
        let tables: Vec<String> = rows
            .iter()
            .map(|row| row.try_get_by_index::<String>(0).unwrap())
            .collect();
        // Mirrors the existing pds/src/db/mod.rs::tests::migrates_actor_db_schema
        // assertion (18 tables, alphabetical, including actor_migrations).
        assert_eq!(
            tables,
            [
                "account_pref",
                "actor_migrations",
                "backlink",
                "blob",
                "record",
                "record_blob",
                "repo_block",
                "repo_root",
                "space_blob_ref",
                "space_def",
                "space_host_reg",
                "space_member",
                "space_oplog",
                "space_record",
                "space_repo",
                "space_repo_notify",
                "space_used_jti",
                "space_writer"
            ]
        );
    }
}
