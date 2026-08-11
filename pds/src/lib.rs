// pds/src/lib.rs
//! cacos PDS library crate.
//!
//! Module layout (Step 1 extraction in progress):
//! - `account`, `actor_store`, `auth`, `oauth`, `observability`, `sequencer`,
//!   `xrpc` are still in-tree; their extraction is planned in Steps 3-9.
//! - `blobstore`, `plc`, `identity`, `handle`, `mailer` were extracted into
//!   sibling workspace members in Step 1; use them via `cacos_pds_<name>::…`.
//! - `error`, `config`, `db`, `background` will move to `cacos-pds-core` in
//!   Step 2.
//! - `context` is being gutted (Step 3 moves the keypair statics into
//!   `cacos-pds-account::auth`, `SharedSequencer` into `cacos-pds-sequencer`).

pub mod account;
pub mod actor_store;
pub mod auth;
pub mod background;
pub mod config;
pub mod context;
pub mod db;
pub mod error;
pub mod oauth;
pub mod observability;
pub mod sequencer;
pub mod xrpc;
                                                                                                                                                                                                                                                                                                                                                 
