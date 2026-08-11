//! DPoP nonce / replay helpers — the low-level hashing and verification
//! reused by `bearer.rs`.
//!
//! Currently empty: the DPoP-side verification is delegated to
//! `rsky_oauth::OAuthProvider` via `validate_dpop_access_token` (in
//! `bearer.rs`). This module exists so future DPoP-specific helpers
//! (nonce cache, replay protection, etc.) have a clear home.
