//! cacos PDS sequencer: typed event sequencer + outbox + apalis-sql worker
//! that backs the `com.atproto.sync.subscribeRepos` firehose.
//!
//! Layer-3 in the planned layered dependency graph:
//!
//! ```text
//!       foundation: cacos-migration, cacos-pds-core
//!                  |
//!       sibling:    cacos-pds-actor-store  (SyncEvtData)
//!                  |
//!            sequencer   (this crate)
//!                  |
//!              server    oauth
//! ```
//!
//! Higher-layer crates (pds / future `cacos-pds-server`, ...) import from
//! this crate; this crate does not import from them.
//!
//! ## Layout (LAYOUT A: flat)
//!
//! - `src/lib.rs` declares the public submodules (`apalis_worker`, `crawlers`,
//!   `db`, `events`, `outbox`, `ws_frames`, `shared_sequencer`) and pulls
//!   the sequencer types in via `include!("mod.rs")`, so callers see
//!   `cacos_pds_sequencer::Sequencer` at the crate root (no nested
//!   `sequencer::` prefix).
//! - `src/mod.rs` holds the `Sequencer` struct (formerly
//!   `pds/src/sequencer/mod.rs`).
//! - `src/shared_sequencer.rs` holds the `Arc<RwLock<Sequencer>>` wrapper
//!   that mounts as poem state.

pub mod apalis_worker;
pub mod crawlers;
pub mod db;
pub mod events;
pub mod outbox;
pub mod shared_sequencer;
pub mod ws_frames;

include!("mod.rs");
