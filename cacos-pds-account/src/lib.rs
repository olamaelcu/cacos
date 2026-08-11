//! cacos PDS account: the `AccountManager` facade plus its helpers
//! (auth helpers, invite codes, passwords, email tokens, admin tokens,
//! PDS signing keypairs, the JWT/DPoP/admin/service-JWT verifier, and the
//! OAuth store).
//!
//! Layer-3 in the planned layered dependency graph:
//!
//! ```text
//!       foundation: cacos-migration, cacos-pds-core
//!                  |
//!                account       (this crate)
//!                  |
//!         +--------+--------+
//!   server       actor-store
//!   oauth        sequencer
//! ```
//!
//! Higher-layer crates (`cacos-pds-server`, `cacos-pds-oauth`,
//! `cacos-pds-actor-store`, `cacos-pds-sequencer`) import from this crate;
//! this crate does not import from them.

pub mod account;
pub mod auth;
