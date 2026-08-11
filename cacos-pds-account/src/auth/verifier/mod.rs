//! Framework-agnostic auth verification.
//!
//! Quirks intentionally fixed (see ADR 0006 track record):
//! 1. `verify_service_jwt_token` measures `now` in **seconds** (RFC 7519
//!    units) and stores `ServiceJwtPayload.exp` as `Duration::from_secs`.
//! 2. `verify_service_jwt` treats `iss` LISTED in `ServiceJwtOpts.iss` as
//!    **trusted** (the inverted-then-`!contains` semantics is the correct
//!    one). Mirrors the OIDC trust check.
//! 3. `verify_service_jwt_token` decodes the signature segment via
//!    base64ct `Base64UrlUnpadded`, hashes the JWS signing input with
//!    SHA-256, and calls `rsky_crypto::verify::verify_signature_digest`
//!    (which expects digest semantics). The previous port re-base64-encoded
//!    the signature and passed the raw signing input.
//! 4. `did_cache` timestamps are reported in **microseconds** to match
//!    `rsky_identity::MemoryCache`; see `cacos_pds_identity::did_cache`.
//!
//! Plus `is_user_or_admin` matches on `r#type == "admin_token"`, not on
//!    `did`, so the admin guard works as documented.

pub mod admin;
pub mod bearer;
pub mod dpop;
pub mod register;
pub mod service_jwt;

// Public API — re-exports preserve the original flat-module surface so
// downstream callers that previously wrote `crate::auth::auth_verifier::X`
// continue to find `X` under `crate::auth::verifier::X`.

// admin.rs
pub use admin::{admin_password_from_env, check_account_status, is_user_or_admin};
pub use admin::{AccountStatus, AdminScopeSet, AdminTokenRegistry};

// bearer.rs
pub use bearer::{
    bearer_token_from_req, dpop_token_from_req, fetch_account_status, is_basic_token,
    is_bearer_token, oauth_scopes_to_auth_scope, parse_basic_auth, validate_access_token,
    validate_bearer_access_token, validate_bearer_token, validate_dpop_access_token,
    validate_refresh_token, verify_admin_token, verify_jwt, verify_user_did_token,
};
pub use bearer::{
    AccessOutput, AuthError, BasicAuth, Credentials, DpopRequestInput, JwtPayload,
    OAuthResponseHeaders, ValidateAccessTokenOpts, ValidatedBearer,
};

// register.rs
pub use register::{
    clear_dpop_request_context, register_auth_dependencies, register_signing_key_resolver,
    set_dpop_request_context, DpopRequestContext, SigningKeyResolver,
};
#[doc(hidden)]
pub use register::_reset_auth_dependencies_for_tests;

// service_jwt.rs
pub use service_jwt::{
    create_service_jwt, verify_service_jwt, ServiceJwtOpts, ServiceJwtPayload, VerifiedServiceJwt,
};
