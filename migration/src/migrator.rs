// migration/src/migrator.rs
//! The four cacos PDS migrators.
//!
//! Each migrator owns one SQLite database file, so each keeps its own applied
//! migration history. The bookkeeping table is named `migrations` (overriding
//! the sea-orm default `seaql_migrations`) so the `pds::db` tests can assert
//! the expected table list per database.
//!
//! `MigratorTrait::up` is called from `pds::db` as `AccountMigrator::up(&db, None)`.

use sea_orm_migration::prelude::*;

use crate::m20260801_000001_account;
use crate::m20260801_000002_repo_seq;
use crate::m20260801_000003_did_doc;
use crate::m20260801_000004_actor;
use crate::m20260801_000005_actor_space;
use crate::m20260801_000006_account_lockout;

/// Account database: 15 tables.
pub struct AccountMigrator;

#[async_trait::async_trait]
impl MigratorTrait for AccountMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260801_000001_account::Migration),
            Box::new(m20260801_000006_account_lockout::Migration),
        ]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("account_migrations").into_iden()
    }
}

/// Sequencer database: `repo_seq`.
pub struct SequencerMigrator;

#[async_trait::async_trait]
impl MigratorTrait for SequencerMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260801_000002_repo_seq::Migration)]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("sequencer_migrations").into_iden()
    }
}

/// DID cache database: `did_doc`.
pub struct DidCacheMigrator;

#[async_trait::async_trait]
impl MigratorTrait for DidCacheMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260801_000003_did_doc::Migration)]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("did_cache_migrations").into_iden()
    }
}

/// Actor database: 17 tables across migrations 001 and 002.
pub struct ActorMigrator;

#[async_trait::async_trait]
impl MigratorTrait for ActorMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260801_000004_actor::Migration),
            Box::new(m20260801_000005_actor_space::Migration),
        ]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("actor_migrations").into_iden()
    }
}
