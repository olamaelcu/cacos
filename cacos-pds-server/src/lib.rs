//! cacos PDS server: the XRPC + OAuth + observability HTTP surface.
//!
//! Layer-4 in the planned layered dependency graph:
//!
//! ```text
//!       foundation: cacos-migration, cacos-pds-core, cacos-pds-account,
//!                   cacos-pds-actor-store, cacos-pds-sequencer,
//!                   cacos-pds-oauth, cacos-pds-blobstore, cacos-pds-plc,
//!                   cacos-pds-identity, cacos-pds-handle, cacos-pds-mailer
//!                  |
//!                server    (this crate)
//!                  |
//!             bin targets (cacos-pds-server, cacos-pds-migrate — planned)
//! ```
//!
//! Higher-layer crates (planned binaries in Step 7/8) import from this crate;
//! this crate does not import from them.

pub mod xrpc;
