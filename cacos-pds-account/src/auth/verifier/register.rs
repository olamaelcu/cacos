//! Registry / lifecycle wiring for auth verification.
//!
//! Holds the OAuth provider, AccountManager, and signing-key resolver
//! used by the rest of the verifier submodules. Tests can reset them
//! via `_reset_auth_dependencies_for_tests`.

use crate::account::AccountManager;
use anyhow::Result;
use rsky_oauth::OAuthProvider;
use std::cell::RefCell;
use std::sync::RwLock;
use std::sync::{Arc, OnceLock};

/// Resolves a `did` (issuer) to its signing key (a did:key string). Wired by
/// the identity plan; returns `InternalServerError` until then.
pub type SigningKeyResolver = Box<dyn Fn(String, bool) -> Result<String> + Send + Sync>;

pub(crate) static OAUTH_PROVIDER: RwLock<Option<Arc<OAuthProvider>>> = RwLock::new(None);
pub(crate) static ACCOUNT_MANAGER: RwLock<Option<Arc<AccountManager>>> = RwLock::new(None);
pub(crate) static SIGNING_KEY_RESOLVER: OnceLock<SigningKeyResolver> = OnceLock::new();
/// Service DID the verifier uses as the audience for inbound service
/// JWTs. Registered at startup from `ServerConfig.service.service_did`
/// so the verifier no longer reads `PDS_SERVICE_DID` from the process
/// environment at request time.
pub(crate) static SERVICE_DID: OnceLock<String> = OnceLock::new();

/// Register the shared OAuth provider (for DPoP-bound access-token
/// validation) and the `AccountManager` (for account-status checks). Called
/// once at startup by the app bootstrap. Bearer-only callers never need the
/// provider.
pub fn register_auth_dependencies(
    account_manager: Arc<AccountManager>,
    provider: Option<Arc<OAuthProvider>>,
) {
    if let Some(provider) = provider
        && let Ok(mut g) = OAUTH_PROVIDER.write()
    {
        *g = Some(provider);
    }
    if let Ok(mut g) = ACCOUNT_MANAGER.write() {
        *g = Some(account_manager);
    }
}

/// Register the service DID so the verifier (and any callers that
/// previously read `PDS_SERVICE_DID` from the environment) can resolve
/// the audience for inbound service JWTs. Called from the XRPC bootstrap
/// after `SharedStateFromEnv::from_env` constructs the typed config.
pub fn register_service_did(did: String) {
    let _ = SERVICE_DID.set(did);
}

/// Returns the registered service DID, or an empty string if the bootstrap
/// did not register one. Replaces the direct `env::var("PDS_SERVICE_DID")`
/// read that the verifier used to do at request time.
pub(crate) fn service_did_from_registry() -> String {
    SERVICE_DID.get().cloned().unwrap_or_default()
}

/// Test-only: clear the registered OAuth provider and AccountManager so
/// a fresh registration can take their place. Production code should
/// never call this.
#[doc(hidden)]
pub fn _reset_auth_dependencies_for_tests() {
    if let Ok(mut g) = OAUTH_PROVIDER.write() {
        *g = None;
    }
    if let Ok(mut g) = ACCOUNT_MANAGER.write() {
        *g = None;
    }
}

/// Register the did -> signing-key resolver used by `verify_user_did_token`
/// and `verify_service_jwt` (wired by the identity plan once IdResolver
/// exists; until then `verify_user_did_token` returns InternalServerError).
pub fn register_signing_key_resolver(
    resolver: SigningKeyResolver,
) -> Result<(), SigningKeyResolver> {
    SIGNING_KEY_RESOLVER.set(resolver)
}

/// The per-request context DPoP proofs bind to (RFC 9449 §4.3): HTTP method,
/// absolute request URI, and all `DPoP` header values.
#[derive(Debug, Clone, Default)]
pub struct DpopRequestContext {
    pub method: String,
    pub uri: String,
    pub dpop_headers: Vec<String>,
}

thread_local! {
    static DPOP_REQUEST_CONTEXT: RefCell<Option<DpopRequestContext>> =
        const { RefCell::new(None) };
}

/// Registers the current request's DPoP context so
/// [`crate::auth::verifier::validate_access_token`] can validate DPoP-bound
/// access tokens.
pub fn set_dpop_request_context(ctx: DpopRequestContext) {
    DPOP_REQUEST_CONTEXT.with(|cell| *cell.borrow_mut() = Some(ctx));
}

pub fn clear_dpop_request_context() {
    DPOP_REQUEST_CONTEXT.with(|cell| *cell.borrow_mut() = None);
}

/// Returns the DPoP request context currently registered for this thread, if
/// any (used by `bearer.rs` to validate DPoP-bound access tokens).
pub(crate) fn current_dpop_request_context() -> Option<DpopRequestContext> {
    DPOP_REQUEST_CONTEXT.with(|cell| cell.borrow().clone())
}
