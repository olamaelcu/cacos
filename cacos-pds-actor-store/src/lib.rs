//! cacos PDS actor store: per-DID storage with a sea-orm-backed sqlite database,
//! a repo layer (block + commit + sync-event types), a record reader (record +
//! backlink tables), a blob reader (metadata + per-DID blobstore), and a
//! preference reader (account_pref rows).
//!
//! Layer-3 in the planned layered dependency graph:
//!
//! ```text
//!       foundation: cacos-migration, cacos-pds-core, cacos-pds-blobstore
//!                  |
//!            actor-store   (this crate)
//!                  |
//!         server    oauth    sequencer
//! ```
//!
//! Higher-layer crates (`cacos-pds-server`, `cacos-pds-oauth`,
//! `cacos-pds-sequencer`) import from this crate; this crate does not import
//! from them.
//!
//! ## Layout (LAYOUT A: flat)
//!
//! - `src/lib.rs` declares the public submodules (`blob`, `db`, `preference`,
//!   `record`, `repo`) and pulls the actor-store types in via
//!   `include!("mod.rs")`, so callers see `cacos_pds_actor_store::ActorStore`
//!   at the crate root (no nested `actor_store::` prefix).
//! - `src/mod.rs` holds the actor-store types (formerly
//!   `pds/src/actor_store/mod.rs`).

pub mod blob;
pub mod db;
pub mod preference;
pub mod record;
pub mod repo;

include!("mod.rs");
