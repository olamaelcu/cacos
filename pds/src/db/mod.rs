//! SQLite connection helpers: open each PDS database and migrate it to the
//! latest schema.
//!
//! Entities and migrators live in the `migration` crate.
use std::path::Path;
use std::time::Duration;

use migration::{
    MigratorTrait,
    migrator::{AccountMigrator, ActorMigrator, DidCacheMigrator, SequencerMigrator},
};
use sea_orm::{
    ConnectOptions, Database, DatabaseConnection, DbErr,
    sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous},
};

/// Re-export the entity modules so `crate::db::entities::<name>` resolves
/// (entities physically live in the `migration` crate; Plans 03-09 import them
/// through this alias, e.g. `crate::db::entities::repo_seq`).
pub use migration::entities;

/// Shared SQLite connection options
fn base_options(url: String) -> ConnectOptions {
    let mut options = ConnectOptions::new(url);
    options
        .map_sqlx_sqlite_opts(|opts| {
            opts.journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Full)
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(3))
        })
        .max_connections(20);
    options
}

/// `sqlite://{path}?mode=rwc` — `mode=rwc` creates the file if missing.
fn sqlite_url(path: impl AsRef<Path>) -> String {
    format!("sqlite://{}?mode=rwc", path.as_ref().display())
}

#[derive(Debug)]
pub enum DatabaseKind {
    Account,
    Sequencer,
    DidCache,
    Actor,
}

impl DatabaseKind {
    pub async fn open(self, path: impl AsRef<Path>) -> std::result::Result<DatabaseConnection, DbErr> {
        let db = Database::connect(base_options(sqlite_url(path))).await?;
        match self {
            Self::Account => AccountMigrator::up(&db, None).await?,
            Self::Actor => ActorMigrator::up(&db, None).await?,
            Self::Sequencer => SequencerMigrator::up(&db, None).await?,
            Self::DidCache => DidCacheMigrator::up(&db, None).await?,
        }

        Ok(db)
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

    use super::*;

    async fn table_names(db: &DatabaseConnection) -> Vec<String> {
        let stmt = Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name"
                .to_owned(),
        );
        let rows = db.query_all_raw(stmt).await.unwrap();
        rows.iter()
            .map(|row| row.try_get_by_index::<String>(0).unwrap())
            .collect()
    }

    async fn index_names(db: &DatabaseConnection, name: &str) -> Vec<String> {
        let stmt = Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("SELECT name FROM sqlite_master WHERE type = 'index' AND name = '{name}'"),
        );
        let rows = db.query_all_raw(stmt).await.unwrap();
        rows.iter()
            .map(|row| row.try_get_by_index::<String>(0).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn migrates_account_db_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseKind::Account
            .open(dir.path().join("account.sqlite"))
            .await
            .unwrap();
        // migrating again is a no-op
        AccountMigrator::up(&db, None).await.unwrap();
        assert_eq!(
            table_names(&db).await,
            [
                "account",
                "account_device",
                "actor",
                "app_password",
                "authorization_request",
                "authorized_client",
                "device",
                "email_token",
                "invite_code",
                "invite_code_use",
                "lexicon",
                "migrations",
                "refresh_token",
                "repo_root",
                "token",
                "used_refresh_token"
            ]
        );
    }

    #[tokio::test]
    async fn migrates_sequencer_db_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseKind::Sequencer
            .open(dir.path().join("sequencer.sqlite"))
            .await
            .unwrap();
        SequencerMigrator::up(&db, None).await.unwrap();
        assert_eq!(table_names(&db).await, ["migrations", "repo_seq"]);
    }

    #[tokio::test]
    async fn migrates_did_cache_db_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseKind::DidCache
            .open(dir.path().join("did_cache.sqlite"))
            .await
            .unwrap();
        DidCacheMigrator::up(&db, None).await.unwrap();
        assert_eq!(table_names(&db).await, ["did_doc", "migrations"]);
    }

    #[tokio::test]
    async fn migrates_actor_db_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseKind::Actor
            .open(dir.path().join("store.sqlite"))
            .await
            .unwrap();
        ActorMigrator::up(&db, None).await.unwrap();
        assert_eq!(
            table_names(&db).await,
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

    #[tokio::test]
    async fn account_db_has_lower_case_unique_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseKind::Account
            .open(dir.path().join("account.sqlite"))
            .await
            .unwrap();
        assert_eq!(
            index_names(&db, "actor_handle_lower_idx").await,
            ["actor_handle_lower_idx"]
        );
        assert_eq!(
            index_names(&db, "account_email_lower_idx").await,
            ["account_email_lower_idx"]
        );
    }

    #[tokio::test]
    async fn sqlite_journal_mode_is_wal() {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseKind::Account
            .open(dir.path().join("account.sqlite"))
            .await
            .unwrap();
        let stmt =
            Statement::from_string(DatabaseBackend::Sqlite, "PRAGMA journal_mode".to_owned());
        let row = db.query_one_raw(stmt).await.unwrap().unwrap();
        let mode: String = row.try_get_by_index(0).unwrap();
        assert_eq!(mode, "wal");
    }
}
