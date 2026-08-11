// pds/src/lib.rs
//! cacos PDS library crate.
//!
//! Module layout (Steps 1-3 complete; later steps in progress):
//! - Step 1 leaves: `blobstore`, `plc`, `identity`, `handle`, `mailer`
//!   (use `cacos_pds_<name>::…`).
//! - Step 2 core: `error`, `config`, `observability`, `db`, `background`
//!   (use `cacos_pds_core::*`).
//! - Step 3 account: `account`, `auth` (use `cacos_pds_account::*`).
//! - Still in pds (Steps 4-8): `actor_store`, `oauth`, `sequencer`,
//!   `xrpc`, and the contents of `context.rs` (`SharedSequencer`).
//!
//! Step 9 will trim this file down to a 1-line stub.

pub mod context;
pub mod oauth;
pub mod xrpc;
