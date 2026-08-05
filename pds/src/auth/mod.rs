//! Auth verifier: pure functions for access/refresh/service JWTs, account
//! status checks, and DPoP-bound OAuth access tokens.
//!
//! Framework-agnostic — this module exposes no poem types. Plan 08's
//! xrpc/auth_extractors.rs wires these functions into poem extractors.
//!
//! `PDS_JWT_KEYPAIR` and `AuthScope` are imported from
//! `crate::account::helpers::auth` (the canonical owner per ADR 0006).

pub mod auth_verifier;
