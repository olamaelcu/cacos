//! SQLite connection helpers: open each PDS database and migrate it to the
//! latest schema.
//!
//! Entities and migrators live in the `migration` crate.
use camino::Utf8Path;
use std::time::Duration;

use migration::{
    MigratorTrait,
    migrator::{AccountMigrator, ActorMigrator, DidCacheMigrator, SequencerMigrator},
};
use sea_orm::{
    ConnectOptions, Database, DatabaseConnection,
    sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous},
};

/// Re-export the entity modules so `crate::db::entities::<name>` resolves
/// (entities physically live in the `migration` crate; Plans 03-09 import them
/// through this alias, e.g. `crate::db::entities::repo_seq`).
pub use migration::entities;

/// Consent-state nonce helpers for the headless-consent remote API.
pub mod consent_state;

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
fn sqlite_url(path: impl AsRef<Utf8Path>) -> String {
    format!("sqlite://{}?mode=rwc", path.as_ref().as_str())
}

#[derive(Debug)]
pub enum DatabaseKind {
    Account,
    Sequencer,
    DidCache,
    Actor,
}

impl DatabaseKind {
    pub async fn open(
        self,
        path: impl AsRef<Utf8Path>,
    ) -> migration::error::Result<DatabaseConnection> {
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

    /// Test-only wrapper: an open PDS database with its backing temp dir kept
    /// alive for the lifetime of the value.
    struct TestDb {
        db: DatabaseConnection,
        _dir: camino_tempfile::Utf8TempDir,
    }

    impl std::ops::Deref for TestDb {
        type Target = DatabaseConnection;

        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }

    /// Test-only helper: open a `DatabaseKind` into a fresh temporary directory.
    trait TestDatabaseKind {
        async fn open_test_db(self) -> TestDb;
    }

    impl TestDatabaseKind for DatabaseKind {
        async fn open_test_db(self) -> TestDb {
            let dir = camino_tempfile::Utf8TempDir::new().unwrap();
            let filename = match self {
                Self::Account => "account.sqlite",
                Self::Sequencer => "sequencer.sqlite",
                Self::DidCache => "did_cache.sqlite",
                Self::Actor => "store.sqlite",
            };
            let db = self.open(dir.path().join(filename)).await.unwrap();
            TestDb { db, _dir: dir }
        }
    }

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
        let db = DatabaseKind::Account.open_test_db().await;
        // migrating again is a no-op
        AccountMigrator::up(&*db, None).await.unwrap();
        assert_eq!(
            table_names(&db).await,
            [
                "account",
                "account_device",
                "account_migrations",
                "actor",
                "app_password",
                "authorization_request",
                "authorized_client",
                "consent_state",
                "device",
                "email_token",
                "invite_code",
                "invite_code_use",
                "lexicon",
                "refresh_token",
                "repo_root",
                "token",
                "used_refresh_token"
            ]
        );
    }

    #[tokio::test]
    async fn migrates_sequencer_db_schema() {
        let db = DatabaseKind::Sequencer.open_test_db().await;
        SequencerMigrator::up(&*db, None).await.unwrap();
        assert_eq!(table_names(&db).await, ["repo_seq", "sequencer_migrations"]);
    }

    #[tokio::test]
    async fn migrates_did_cache_db_schema() {
        let db = DatabaseKind::DidCache.open_test_db().await;
        DidCacheMigrator::up(&*db, None).await.unwrap();
        assert_eq!(table_names(&db).await, ["did_cache_migrations", "did_doc"]);
    }

    #[tokio::test]
    async fn migrates_actor_db_schema() {
        let db = DatabaseKind::Actor.open_test_db().await;
        ActorMigrator::up(&*db, None).await.unwrap();
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
        let db = DatabaseKind::Account.open_test_db().await;
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
        let db = DatabaseKind::Account.open_test_db().await;
        let stmt =
            Statement::from_string(DatabaseBackend::Sqlite, "PRAGMA journal_mode".to_owned());
        let row = db.query_one_raw(stmt).await.unwrap().unwrap();
        let mode: String = row.try_get_by_index(0).unwrap();
        assert_eq!(mode, "wal");
    }

    #[tokio::test]
    async fn custom_types_roundtrip() {
        use sea_orm::{ActiveModelTrait, EntityTrait, Set};

        let db = DatabaseKind::Sequencer.open_test_db().await;

        let test_did: migration::types::did::Did = "did:plc:test123".parse().unwrap();
        let test_seq = migration::types::db_id::DbId::new();
        let now = time::OffsetDateTime::now_utc();

        // Insert into repo_seq — exercises DbId PK, Did, OffsetDateTime, Vec<u8>
        let model = entities::repo_seq::ActiveModel {
            seq: Set(test_seq),
            did: Set(test_did.clone()),
            event_type: Set("test".to_owned()),
            event: Set(b"test event".to_vec()),
            invalidated: Set(Some(0)),
            sequenced_at: Set(now),
        };
        let res = model.insert(&*db).await.unwrap();
        assert_eq!(res.seq, test_seq);
        assert_eq!(res.did, test_did);
        assert_eq!(res.event, b"test event");

        // Read it back
        let fetched = entities::repo_seq::Entity::find_by_id(test_seq)
            .one(&*db)
            .await
            .unwrap()
            .expect("should find inserted row");
        assert_eq!(fetched.did, test_did);
        assert_eq!(fetched.event_type, "test");
        assert_eq!(fetched.sequenced_at, now);
    }
}
