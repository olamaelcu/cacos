// migration/src/lib.rs
//! sea-orm migration crate for the cacos PDS.
//!
//! Holds the entities and migrators for all four PDS SQLite databases:
//! account, sequencer, did-cache, and actor. The `pds` crate imports the
//! migrators as `migration::migrator::{AccountMigrator, SequencerMigrator,
//! DidCacheMigrator, ActorMigrator}` and runs them from `pds::db`.

pub use sea_orm_migration::prelude::*;

pub mod entities;
pub mod migrator;

pub mod m20260801_000001_account;
pub mod m20260801_000002_repo_seq;
pub mod m20260801_000003_did_doc;
pub mod m20260801_000004_actor;
pub mod m20260801_000005_actor_space;
