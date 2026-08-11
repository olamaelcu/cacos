//! Shared OAuth provider module: `SharedOAuthProvider`, the SSRF-hardened
//! client-metadata fetcher (Task 6), and the headless-consent remote API
//! (Task 5).
//!
//! [`SharedOAuthProvider::new`] constructs an `OAuthProvider` from env
//! (`PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`, `PDS_DPOP_SECRET`, and
//! `PDS_OAUTH_TRUSTED_CLIENTS`) and wires it to the cacos backing store
//! ([`crate::account::oauth_store::PdsOAuthStore`]).

pub mod csrf;
pub mod fetcher;
pub mod remote;
pub mod remote_create_account;
pub mod routes;

use crate::account::oauth_store::PdsOAuthStore;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
#[allow(unused_imports)] // clippy false-positive: used by build_oauth_app's `.data()`
use poem::EndpointExt;
use poem::web::cookie::{Cookie, CookieJar, SameSite};
use rsky_oauth::dpop::{DEFAULT_ROTATION_INTERVAL, DpopManager, DpopNonce, ReplayStore};
use rsky_oauth::jwk::{EcCurve, Jwk};
use rsky_oauth::store::DeviceData;
use rsky_oauth::{OAuthError, OAuthProvider, OAuthProviderConfig};
use sea_orm::DatabaseConnection;
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

/// Stub that Task 6 replaces with the real SSRF-hardened fetcher.
pub use fetcher::HttpClientMetadataFetcher;

pub const DEVICE_COOKIE: &str = "device-id";

/// Build the OAuth route set plus a handle to the [`OAuthProvider`] so the
/// XRPC bootstrap can register it for DPoP-bound access-token validation.
///
/// `endpoint` is generic over the concrete route type returned by
/// [`build_oauth_app`] because `poem::Endpoint` is not dyn-compatible.
/// Callers consume the struct inline (do not store it in a `Box<dyn>` or
/// similar trait object) — the type is inferred at the use site.
pub struct OAuthBootstrap<E> {
    pub endpoint: E,
    pub provider: Arc<rsky_oauth::OAuthProvider>,
}

/// The handle to the most recently registered OAuth provider. Set inside
/// [`bootstrap_oauth_app`]; consumed by integration tests that need to
/// drive the full PAR → token flow against the same provider instance
/// the resource server has registered for DPoP-bound access-token
/// validation. Cleared via `_reset_auth_dependencies_for_tests` /
/// [`_reset_registered_provider_for_tests`].
static REGISTERED_PROVIDER: OnceLock<Arc<OAuthProvider>> = OnceLock::new();

/// Test-only: read the most recently registered OAuth provider so an
/// integration test can mint DPoP-bound access tokens against the same
/// store the resource server validates against.
#[doc(hidden)]
pub fn registered_provider() -> Option<Arc<OAuthProvider>> {
    REGISTERED_PROVIDER.get().cloned()
}

/// the browser binds it to the host (no Domain attribute, Path=/,
/// Secure). On plain HTTP we fall back to the legacy `device-id` name
/// because `__Host-` requires Secure.
pub fn device_cookie_name(public_url: &str) -> &'static str {
    if public_url.starts_with("https://") {
        "__Host-device-id"
    } else {
        DEVICE_COOKIE
    }
}

/// Whether the device cookie should be marked Secure. Aligned with
/// [`device_cookie_name`]: HTTPS deployments set Secure; plain HTTP
/// cannot, because Secure cookies never ride on a non-TLS origin.
pub fn device_cookie_secure(public_url: &str) -> bool {
    public_url.starts_with("https://")
}

/// Lazily-built `HashSet<String>` of `PDS_OAUTH_TRUSTED_CLIENTS` values,
/// used by `is_trusted_oauth_client` for constant-time membership checks.
fn trusted_clients_set() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        std::env::var("PDS_OAUTH_TRUSTED_CLIENTS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Returns `true` when `candidate` is in the configured
/// `PDS_OAUTH_TRUSTED_CLIENTS` set. Uses `subtle::ConstantTimeEq` so the
/// comparison does not leak length or content via timing.
pub fn is_trusted_oauth_client(candidate: &str) -> bool {
    let candidate_bytes = candidate.as_bytes();
    trusted_clients_set()
        .iter()
        .any(|registered| registered.as_bytes().ct_eq(candidate_bytes).into())
}

/// Shared OAuth provider handle, mounted as poem state. Plan 08 reads this
/// from the request to validate DPoP-bound access tokens.
#[derive(Clone)]
pub struct SharedOAuthProvider {
    pub provider: Arc<OAuthProvider>,
}

impl SharedOAuthProvider {
    pub fn new(
        account_db: DatabaseConnection,
        issuer: String,
        audience: String,
        replay_store: Box<dyn ReplayStore>,
    ) -> Self {
        let private_key = std::env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX")
            .expect("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX must be set");
        let key_bytes = hex::decode(private_key).expect("invalid provider signing key hex");
        let signing_key = Jwk::from_private_key_bytes(EcCurve::K256, &key_bytes)
            .expect("invalid provider signing key");
        let nonce = match std::env::var("PDS_DPOP_SECRET") {
            Ok(secret_hex) => {
                let secret: [u8; 32] = hex::decode(secret_hex)
                    .expect("PDS_DPOP_SECRET must be hex")
                    .try_into()
                    .expect("PDS_DPOP_SECRET must be 32 bytes");
                let mut secret_box = SecretBox::new(Box::new(secret));
                let nonce = DpopNonce::new(*secret_box.expose_secret(), DEFAULT_ROTATION_INTERVAL);
                secret_box.expose_secret_mut().zeroize();
                nonce
            }
            Err(_) => DpopNonce::new_random(DEFAULT_ROTATION_INTERVAL),
        }
        .expect("valid DPoP nonce rotation interval");
        let trusted_clients: Vec<String> = std::env::var("PDS_OAUTH_TRUSTED_CLIENTS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        // Touch the lazy-cached set so it is materialised alongside the
        // provider (avoids a cold-start miss in callers using
        // `is_trusted_oauth_client`).
        let _ = trusted_clients_set();
        let provider = OAuthProvider::new(OAuthProviderConfig {
            issuer,
            audience,
            signing_key,
            fetcher: Arc::new(HttpClientMetadataFetcher::new()),
            store: Arc::new(PdsOAuthStore::new(account_db)),
            dpop: DpopManager::new(Some(nonce), replay_store),
            trusted_clients,
        });
        Self {
            provider: Arc::new(provider),
        }
    }
}

/// Absolute public base URL used to build DPoP `htu` values in the routes.
/// Plan 08 passes the real `ServerConfig.service.public_url` when mounting.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub public_url: String,
}

/// Spawn the periodic DPoP-replay-row prune. Each tick deletes rows whose
/// `expiresAt` is in the past and increments
/// `cacos_dpop_replay_pruned_total` by the row count. The five-minute
/// interval is short enough that the table stays bounded even under
/// sustained traffic, but long enough that the per-tick cost is
/// negligible against the indexed column.
fn schedule_dpop_replay_prune(store: Arc<crate::account::oauth_store::DbBackedReplayStore>) {
    let queue = crate::background::BackgroundQueue::default();
    let store = store.clone();
    queue.add(async move {
        let interval = std::time::Duration::from_secs(5 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let now = now_secs();
            match store.prune_expired(now).await {
                Ok(0) => {}
                Ok(pruned) => {
                    metrics::counter!(crate::observability::metrics::DPOP_REPLAY_PRUNED_TOTAL)
                        .increment(pruned as u64);
                    tracing::debug!(pruned, "dpop replay rows pruned");
                }
                Err(err) => {
                    tracing::warn!(?err, "dpop replay prune failed");
                }
            }
        }
    });
    // Keep the queue alive for the lifetime of the process by leaking it;
    // the spawned task is the only owner and it runs forever.
    std::mem::forget(queue);
}

/// Environment-driven bootstrap for the OAuth route set, given an
/// already-opened (and migrated) account DB. Returns `None` when
/// `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX` is unset (the server still runs with
/// `/metrics` only).
///
/// Env:
/// - `PDS_PUBLIC_URL` (default `http://localhost:8080`): absolute base URL
///   used to build DPoP `htu` values and OAuth metadata.
/// - `PDS_OAUTH_REMOTE_CLIENT_URL` / `PDS_OAUTH_REMOTE_CLIENT_TOKEN`:
///   headless-consent RemoteClient config (see [`crate::config`]).
pub fn bootstrap_oauth_app(
    account_db: sea_orm::DatabaseConnection,
    account_manager: crate::account::AccountManager,
    actor_store: std::sync::Arc<crate::actor_store::ActorStore>,
    plc_client: std::sync::Arc<dyn crate::plc::PlcClient>,
) -> Option<OAuthBootstrap<impl poem::Endpoint<Output = poem::Response>>> {
    if std::env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX").is_err() {
        return None;
    }
    let public_url =
        std::env::var("PDS_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let issuer = public_url.clone();
    let audience =
        std::env::var("PDS_SERVICE_DID").unwrap_or_else(|_| "did:web:localhost".to_string());

    let replay_store = Arc::new(crate::account::oauth_store::DbBackedReplayStore::new(
        std::sync::Arc::new(account_db.clone()),
    ));
    schedule_dpop_replay_prune(replay_store.clone());
    // Clone the store (cheap Arc clone of the inner DB handle) so the same
    // logical store drives both the prune task and the DPoP manager. The
    // DpopManager takes `Box<dyn ReplayStore>` because rsky-oauth's trait
    // methods are sync.
    let shared = SharedOAuthProvider::new(
        account_db.clone(),
        issuer,
        audience,
        Box::new((*replay_store).clone()),
    );
    let provider = Arc::clone(&shared.provider);
    // Publish to the module-level handle so tests can mint tokens against
    // the same provider the resource server registered.
    let _ = REGISTERED_PROVIDER.set(Arc::clone(&provider));
    let remote_config = crate::config::OAuthRemoteConfig::from_env();
    let remote_create_account: Arc<dyn remote_create_account::RemoteCreateAccount> =
        Arc::new(remote_create_account::ActorStoreRemoteCreateAccount::new(
            account_manager,
            actor_store,
            plc_client,
        ));
    let endpoint = build_oauth_app(
        shared,
        account_db,
        remote_config,
        public_url,
        remote_create_account,
    );
    Some(OAuthBootstrap { endpoint, provider })
}

/// Builds the full OAuth route set (provider routes + authorize redirect +
/// headless-consent remote API), registering the shared provider, account
/// DB, remote config, public-URL config, and the `RemoteCreateAccount` impl
/// as poem data. Plan 08 swaps in the real `RemoteCreateAccount`.
pub fn build_oauth_app(
    shared: SharedOAuthProvider,
    account_db: sea_orm::DatabaseConnection,
    remote_config: crate::config::OAuthRemoteConfig,
    public_url: String,
    remote_create_account: Arc<dyn remote_create_account::RemoteCreateAccount>,
) -> impl poem::Endpoint<Output = poem::Response> {
    use crate::xrpc::rate_limit::{RouteRateLimit, ip_limiter};
    use poem::{Middleware, get, post};

    // The headless-consent POSTs relay credentials (`sign-in`,
    // `create-account`) and mutate authorization state, so they carry a
    // per-IP budget on top of the bearer-token guard. The middleware runs
    // before `TokenGuard`, so unauthenticated floods are shed too.
    //
    // NOTE: legitimate traffic here originates from the single configured
    // RemoteClient, so every request shares one source IP. Operators
    // fronting a busy PDS must raise `PDS_RATELIMIT_OAUTH_REMOTE_PER_MINUTE`
    // — and 0 does not disable the limiter, `ip_limiter` clamps it to 1/min.
    let remote_rl = RouteRateLimit {
        limiter: ip_limiter(
            std::env::var("PDS_RATELIMIT_OAUTH_REMOTE_PER_MINUTE")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(10),
        ),
    };

    poem::Route::new()
        .at("/oauth/par", post(routes::oauth_par))
        .at("/oauth/token", post(routes::oauth_token))
        .at("/oauth/revoke", post(routes::oauth_revoke))
        .at("/oauth/jwks", get(routes::oauth_jwks))
        .at(
            "/.well-known/oauth-authorization-server",
            get(routes::oauth_authorization_server_metadata),
        )
        .at(
            "/.well-known/oauth-protected-resource",
            get(routes::oauth_protected_resource_metadata),
        )
        .at(
            "/oauth/authorize/:client_id/:request_uri",
            get(routes::oauth_authorize),
        )
        .at("/oauth/remote/request", get(remote::request))
        .at(
            "/oauth/remote/sign-in",
            post(remote_rl.transform(remote::sign_in)),
        )
        .at(
            "/oauth/remote/select",
            post(remote_rl.transform(remote::select_account)),
        )
        .at(
            "/oauth/remote/create-account",
            post(remote_rl.transform(remote::create_account)),
        )
        .at(
            "/oauth/remote/accept",
            post(remote_rl.transform(remote::accept)),
        )
        .at(
            "/oauth/remote/reject",
            post(remote_rl.transform(remote::reject)),
        )
        .data(shared)
        .data(account_db)
        .data(remote_config)
        .data(OAuthConfig { public_url })
        .data(remote_create_account)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

fn random_prefixed_id(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        hex::encode(rsky_crypto::utils::random_bytes(16))
    )
}

/// The CSRF token for a device session is derived from the HttpOnly
/// cookie value, which page scripts and other origins cannot read.
pub fn csrf_token(cookie_value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(cookie_value.as_bytes()))
}

/// The authenticated device session for the authorization UI.
pub struct DeviceSession {
    pub device_id: String,
    pub csrf: String,
}

/// Loads the device session from the cookie, creating a fresh device row
/// (and cookie) when absent or invalid.
pub async fn ensure_device_session(
    store: &dyn rsky_oauth::store::OAuthStore,
    jar: &CookieJar,
    user_agent: Option<&str>,
    ip_address: &str,
    now: u64,
    public_url: &str,
) -> Result<DeviceSession, OAuthError> {
    if let Some(cookie) = jar.get(device_cookie_name(public_url)) {
        let value = cookie.value_str().to_string();
        if let Some((device_id, session_id)) = value.split_once('.')
            && let Some(device) = store.read_device(device_id).await?
            && device.session_id == session_id
        {
            return Ok(DeviceSession {
                device_id: device_id.to_string(),
                csrf: csrf_token(&value),
            });
        }
    }
    let device_id = random_prefixed_id("dev-");
    let session_id = random_prefixed_id("ses-");
    store
        .create_device(
            &device_id,
            &DeviceData {
                session_id: session_id.clone(),
                user_agent: user_agent.map(String::from),
                ip_address: ip_address.to_string(),
                last_seen_at: now,
            },
        )
        .await?;
    let value = format!("{device_id}.{session_id}");
    let csrf = csrf_token(&value);
    let mut cookie = Cookie::new_with_str(device_cookie_name(public_url), value);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    // `__Host-` cookies (RFC 6265bis) must use Path=/ and Secure, with no
    // Domain attribute. The Plain-HTTP fallback also uses Path=/ so the
    // cookie rides on the same path the user agent already had.
    cookie.set_path("/".to_string());
    cookie.set_secure(device_cookie_secure(public_url));
    jar.add(cookie);
    Ok(DeviceSession { device_id, csrf })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsky_oauth::store::{AccountInfo, MemoryOAuthStore};
    use rsky_oauth::{OAuthProvider, OAuthProviderConfig};

    const TEST_KEY_HEX: &str = "4242424242424242424242424242424242424242424242424242424242424242";
    const ISSUER: &str = "https://pds.test";
    const AUDIENCE: &str = "did:web:pds.test";

    fn init_provider_env() {
        let defaults = [
            ("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX", TEST_KEY_HEX),
            ("PDS_DPOP_SECRET", TEST_KEY_HEX),
            ("PDS_OAUTH_TRUSTED_CLIENTS", ""),
            ("PDS_SERVICE_DID", "did:web:pds.test"),
        ];
        for (key, value) in defaults {
            // SAFETY: tests run sequentially within a process.
            unsafe { std::env::set_var(key, value) };
        }
    }

    #[tokio::test]
    async fn constructs_provider_from_env() {
        init_provider_env();
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = sea_orm::Database::connect(format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("x.sqlite").as_str()
        ))
        .await
        .unwrap();
        let shared = SharedOAuthProvider::new(
            db,
            ISSUER.to_string(),
            AUDIENCE.to_string(),
            Box::new(rsky_oauth::InMemoryReplayStore::default()),
        );
        let provider = &shared.provider;
        assert_eq!(provider.issuer(), ISSUER);
        assert_eq!(provider.jwks().keys.len(), 1);
        let now = now_secs();
        assert!(provider.next_dpop_nonce(now).is_some());
    }

    #[test]
    fn csrf_token_is_deterministic_and_opaque() {
        assert_eq!(csrf_token("cookie-1"), csrf_token("cookie-1"));
        assert_ne!(csrf_token("cookie-1"), csrf_token("cookie-2"));
        assert!(!csrf_token("cookie-1").contains("cookie-1"));
    }

    #[test]
    fn device_cookie_uses_host_prefix_and_secure_when_https() {
        assert_eq!(
            device_cookie_name("https://pds.example"),
            "__Host-device-id"
        );
        assert!(device_cookie_secure("https://pds.example"));
    }

    #[test]
    fn device_cookie_uses_legacy_name_and_insecure_when_http() {
        assert_eq!(device_cookie_name("http://localhost:2583"), "device-id");
        assert!(!device_cookie_secure("http://localhost:2583"));
    }

    fn memory_provider() -> Arc<OAuthProvider> {
        let store = Arc::new(MemoryOAuthStore::new());
        store.add_account(
            AccountInfo {
                did: "did:plc:alice".to_string(),
                handle: Some("alice.example.com".to_string()),
                email: None,
                deactivated: false,
            },
            "correct-password",
        );
        let key = rsky_oauth::Jwk::from_private_key_bytes(
            rsky_oauth::EcCurve::K256,
            &hex::decode(TEST_KEY_HEX).unwrap(),
        )
        .unwrap();
        Arc::new(OAuthProvider::new(OAuthProviderConfig {
            issuer: ISSUER.to_string(),
            audience: AUDIENCE.to_string(),
            signing_key: key,
            fetcher: Arc::new(super::fetcher::HttpClientMetadataFetcher::new()),
            store,
            dpop: rsky_oauth::DpopManager::new(
                None,
                Box::new(rsky_oauth::InMemoryReplayStore::default()),
            ),
            trusted_clients: vec![],
        }))
    }

    #[tokio::test]
    async fn ensure_device_session_creates_and_validates() {
        init_provider_env();
        let provider = memory_provider();
        let now = now_secs();
        let jar = CookieJar::default();
        let session = ensure_device_session(
            provider.store().as_ref(),
            &jar,
            Some("agent"),
            "127.0.0.1",
            now,
            "https://pds.test",
        )
        .await
        .unwrap();
        assert!(session.device_id.starts_with("dev-"));
        let cookie = jar
            .get(device_cookie_name("https://pds.test"))
            .expect("cookie set")
            .value_str()
            .to_string();
        assert!(cookie.starts_with("dev-"));
        assert_eq!(session.csrf, csrf_token(&cookie));

        let session2 = ensure_device_session(
            provider.store().as_ref(),
            &jar,
            Some("agent"),
            "127.0.0.1",
            now,
            "https://pds.test",
        )
        .await
        .unwrap();
        assert_eq!(session2.device_id, session.device_id);
        assert_eq!(session2.csrf, session.csrf);

        let jar2 = CookieJar::default();
        let mut tampered = Cookie::new_with_str(
            device_cookie_name("https://pds.test"),
            "dev-tampered.ses-tampered",
        );
        tampered.set_http_only(true);
        tampered.set_same_site(SameSite::Lax);
        tampered.set_path("/".to_string());
        tampered.set_secure(true);
        jar2.add(tampered);
        let session3 = ensure_device_session(
            provider.store().as_ref(),
            &jar2,
            None,
            "127.0.0.1",
            now,
            "https://pds.test",
        )
        .await
        .unwrap();
        assert_ne!(session3.device_id, session.device_id);
    }
}
