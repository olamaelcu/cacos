//! cacos PDS OAuth: the headless-consent OAuth provider, the SSRF-hardened
//! client-metadata fetcher, the remote API surface, and the per-IP rate
//! limit middleware that fronts the remote endpoints.
//!
//! Layer-4 in the planned layered dependency graph:
//!
//! ```text
//!       foundation: cacos-migration, cacos-pds-core, cacos-pds-account
//!                  |
//!                 oauth    (this crate)
//!                  |
//!                server
//! ```
//!
//! Higher-layer crates (`cacos-pds-server`) import from this crate; this
//! crate does not import from them. The crate exposes a `bootstrap_oauth_app`
//! function plus a `SharedOAuthProvider` handle so the server can register
//! the provider for DPoP-bound access-token validation, and a route set
//! that the server mounts under `/oauth/*` and `/.well-known/oauth-*`.
//!
//! ## Layout (LAYOUT A: flat)
//!
//! - `src/lib.rs` declares the public submodules (`fetcher`, `rate_limit`,
//!   `remote`, `remote_create_account`, `routes`) and pulls the body of
//!   the OAuth provider via `include!("mod.rs")`, so callers see
//!   `cacos_pds_oauth::SharedOAuthProvider` at the crate root.
//! - `src/mod.rs` holds the OAuth provider state-machine (env-var loading,
//!   `bootstrap_oauth_app`, `now_secs`, the SSRF-stub `HttpClientMetadataFetcher`
//!   re-export, ...).

pub mod fetcher;
pub mod rate_limit;
pub mod remote;
pub mod remote_create_account;
pub mod routes;

include!("mod.rs");
