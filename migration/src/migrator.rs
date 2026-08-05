// migration/src/migrator.rs
//! The four cacos PDS migrators.
//!
//! Each migrator owns one SQLite database file, so each keeps its own applied
//! migration history. The bookkeeping table is named `migrations` (overriding
//! the sea-orm default `seaql_migrations`) so the table lists asserted by the
//! `pds::db` tests match the reference rsky test expectations.
//!
//! `MigratorTrait::up` is called from `pds::db` as `AccountMigrator::up(&db, None)`
//! (see verified note 1).

use sea_orm_migration::prelude::*;

use crate::m20260801_000001_account;

/// Account database: 15 tables (port of `account_manager/db.rs` in rsky).
pub struct AccountMigrator;

#[async_trait::async_trait]
impl MigratorTrait for AccountMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260801_000001_account::Migration)]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("migrations").into_iden()
    }
}

/// Sequencer database: `repo_seq` (port of `sequencer/db.rs` in rsky).
pub struct SequencerMigrator;

#[async_trait::async_trait]
impl MigratorTrait for SequencerMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("migrations").into_iden()
    }
}

/// DID cache database: `did_doc` (port of `did_cache.rs` in rsky).
pub struct DidCacheMigrator;

#[async_trait::async_trait]
impl MigratorTrait for DidCacheMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("migrations").into_iden()
    }
}

/// Actor database: 17 tables across migrations 001 and 002
/// (port of `actor_store/db/mod.rs` in rsky).
pub struct ActorMigrator;

#[async_trait::async_trait]
impl MigratorTrait for ActorMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("actor_migrations").into_iden()
    }
}
