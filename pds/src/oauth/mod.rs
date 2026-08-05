//! Shared OAuth provider module: `SharedOAuthProvider`, the SSRF-hardened
//! client-metadata fetcher (Task 6), and the headless-consent remote API
//! (Task 5).
//!
//! [`SharedOAuthProvider::new`] constructs an `OAuthProvider` from env
//! (`PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`, `PDS_DPOP_SECRET`, and
//! `PDS_OAUTH_TRUSTED_CLIENTS`) and wires it to the cacos backing store
//! ([`crate::account::oauth_store::PdsOAuthStore`]).

pub mod fetcher;

use crate::account::oauth_store::PdsOAuthStore;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use poem::web::cookie::{Cookie, CookieJar, SameSite};
use rsky_oauth::dpop::{DpopManager, DpopNonce, InMemoryReplayStore, DEFAULT_ROTATION_INTERVAL};
use rsky_oauth::jwk::{EcCurve, Jwk};
use rsky_oauth::store::DeviceData;
use rsky_oauth::{OAuthError, OAuthProvider, OAuthProviderConfig};
use sea_orm::DatabaseConnection;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Stub that Task 6 replaces with the real SSRF-hardened fetcher.
pub use fetcher::HttpClientMetadataFetcher;

pub const DEVICE_COOKIE: &str = "device-id";

/// Shared OAuth provider handle, mounted as poem state. Plan 08 reads this
/// from the request to validate DPoP-bound access tokens.
#[derive(Clone)]
pub struct SharedOAuthProvider {
    pub provider: Arc<OAuthProvider>,
}

impl SharedOAuthProvider {
    pub fn new(account_db: DatabaseConnection, issuer: String, audience: String) -> Self {
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
                DpopNonce::new(secret, DEFAULT_ROTATION_INTERVAL)
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
        let provider = OAuthProvider::new(OAuthProviderConfig {
            issuer,
            audience,
            signing_key,
            fetcher: Arc::new(HttpClientMetadataFetcher::new()),
            store: Arc::new(PdsOAuthStore::new(account_db)),
            dpop: DpopManager::new(Some(nonce), Box::new(InMemoryReplayStore::default())),
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
) -> Result<DeviceSession, OAuthError> {
    if let Some(cookie) = jar.get(DEVICE_COOKIE) {
        let value = cookie.value_str().to_string();
        if let Some((device_id, session_id)) = value.split_once('.') {
            if let Some(device) = store.read_device(device_id).await? {
                if device.session_id == session_id {
                    return Ok(DeviceSession {
                        device_id: device_id.to_string(),
                        csrf: csrf_token(&value),
                    });
                }
            }
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
    let mut cookie = Cookie::new_with_str(DEVICE_COOKIE, value);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/oauth".to_string());
    jar.add(cookie);
    Ok(DeviceSession { device_id, csrf })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsky_oauth::store::{AccountInfo, MemoryOAuthStore};
    use rsky_oauth::{OAuthProvider, OAuthProviderConfig};

    const TEST_KEY_HEX: &str =
        "4242424242424242424242424242424242424242424242424242424242424242";
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
        let shared = SharedOAuthProvider::new(db, ISSUER.to_string(), AUDIENCE.to_string());
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
        let session = ensure_device_session(provider.store().as_ref(), &jar, Some("agent"), "127.0.0.1", now)
            .await
            .unwrap();
        assert!(session.device_id.starts_with("dev-"));
        let cookie = jar
            .get(DEVICE_COOKIE)
            .expect("cookie set")
            .value_str()
            .to_string();
        assert!(cookie.starts_with("dev-"));
        assert_eq!(session.csrf, csrf_token(&cookie));

        let session2 = ensure_device_session(provider.store().as_ref(), &jar, Some("agent"), "127.0.0.1", now)
            .await
            .unwrap();
        assert_eq!(session2.device_id, session.device_id);
        assert_eq!(session2.csrf, session.csrf);

        let jar2 = CookieJar::default();
        let mut tampered = Cookie::new_with_str(DEVICE_COOKIE, "dev-tampered.ses-tampered");
        tampered.set_http_only(true);
        tampered.set_same_site(SameSite::Lax);
        tampered.set_path("/oauth".to_string());
        jar2.add(tampered);
        let session3 = ensure_device_session(provider.store().as_ref(), &jar2, None, "127.0.0.1", now)
            .await
            .unwrap();
        assert_ne!(session3.device_id, session.device_id);
    }
}
