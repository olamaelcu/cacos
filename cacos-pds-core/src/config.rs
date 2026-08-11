//! Server configuration loaded from the environment.

use std::collections::HashSet;
use std::env;

/// Headless-consent RemoteClient configuration.
///
/// A single operator-configured RemoteClient owns rendering for the OAuth
/// authorize/sign-in/consent/error screens; the PDS exposes a
/// token-authenticated JSON API instead. Unset `url`/`token` disables the
/// `/oauth/authorize` redirect (it returns 503).
#[derive(Debug, Clone, Default)]
pub struct OAuthRemoteConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

impl OAuthRemoteConfig {
    pub fn from_env() -> Self {
        let url = env::var("PDS_OAUTH_REMOTE_CLIENT_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let token = env::var("PDS_OAUTH_REMOTE_CLIENT_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        Self { url, token }
    }
}

/// Top-level service configuration: the public hostname, the absolute URL,
/// the bind port for the poem server, and the service DID that audiences
/// of `cacos_pds_account` JWTs and the `atproto_pds` service endpoint
/// resolve to. The DID is stored on the struct so handlers and
/// `AccountManager` callers never have to read `PDS_SERVICE_DID` from
/// the environment at request time.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub hostname: String,
    pub public_url: String,
    pub port: u16,
    pub service_did: String,
}

/// On-disk paths for each of the three PDS SQLite databases.
#[derive(Debug, Clone)]
pub struct ServiceDbConfig {
    pub account_db_location: String,
    pub sequencer_db_location: String,
    pub did_cache_db_location: String,
}

/// Identity resolver tuning.
#[derive(Debug, Clone)]
pub struct IdentityConfig {
    pub plc_url: String,
    pub resolver_timeout: u64,
    pub service_handle_domains: Vec<String>,
    /// Selects the production PLC client. `http` (the default) uses the
    /// SSRF-guarded `HttpPlcClient`; `mock` is only available with the
    /// `test-utils` feature on `cacos_pds_plc` and is what integration
    /// tests reach for when the production directory would need network
    /// access. Mirrors the previous `PDS_PLC_CLIENT_MODE` env var.
    pub plc_client_mode: String,
}

/// CORS origin allowlist. Parsed from `PDS_CORS_ALLOWED_ORIGINS` as a
/// comma-separated list of exact origins; an empty set falls back to
/// echoing the configured `public_url` origin in the response.
#[derive(Debug, Clone, Default)]
pub struct CorsConfig {
    pub allowed_origins: HashSet<String>,
}

/// Health/version reporting. `version` is surfaced at `/xrpc/_health`.
#[derive(Debug, Clone)]
pub struct HealthConfig {
    pub version: String,
}

/// Headless-consent OAuth provider tuning.
///
/// `trusted_clients` is the always-allow list of OAuth client IDs that
/// the provider recognises; the default of an empty list means the
/// per-client allow-screen renders for every authorization request.
/// `rate_limit_per_minute` is the per-IP token-bucket budget for the
/// headless-consent remote POSTs (`sign-in`, `create-account`,
/// `accept`, `reject`); the default mirrors the historical
/// `unwrap_or_else(|_| 50u32)` in `cacos_pds_oauth` before the rate
/// limit was lifted to config.
#[derive(Debug, Clone)]
pub struct OauthConfig {
    pub trusted_clients: Vec<String>,
    pub rate_limit_per_minute: u32,
}

/// Server-side rate-limit budgets for the XRPC write endpoints. Each
/// value is a per-IP, per-minute token-bucket cap. The defaults mirror
/// `cacos_pds_server::xrpc::mod.rs` before they were lifted to config
/// (createSession/createAccount = 10, passwordReset/emailOps = 5).
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub create_session_per_minute: u32,
    pub create_account_per_minute: u32,
    pub password_reset_per_minute: u32,
    pub email_ops_per_minute: u32,
}

/// Per-actor blob store configuration. The blobstore is built once at
/// process startup (`cacos_pds_blobstore::init_operator`) and the per-DID
/// handle is handed out at actor construction time; `disk_location`
/// captures the filesystem backend root (when S3 is not configured).
#[derive(Debug, Clone)]
pub struct BlobstoreConfig {
    pub disk_location: String,
}

/// Assembled runtime configuration. `env_to_cfg` reads every field from the
/// process environment; defaults mirror the rsky reference and are safe for
/// local development.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub service: ServiceConfig,
    pub service_db: ServiceDbConfig,
    pub identity: IdentityConfig,
    pub actor_store: ActorStoreConfig,
    pub blobstore: BlobstoreConfig,
    pub cors: CorsConfig,
    pub health: HealthConfig,
    pub oauth: OauthConfig,
    pub rate_limit: RateLimitConfig,
    pub crawlers: Vec<String>,
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn parse_port(raw: &str) -> u16 {
    raw.parse::<u16>()
        .unwrap_or_else(|_| panic!("invalid port value: {raw}"))
}

/// Read the full server configuration from the process environment. Every
/// field has a default so the binary boots without a `.env` file in
/// development.
pub fn env_to_cfg() -> ServerConfig {
    let port = parse_port(&env_or("PDS_PORT", "8080"));
    let hostname = env_or("PDS_HOSTNAME", "localhost");
    let public_url = env_or("PDS_PUBLIC_URL", &format!("http://{hostname}:{port}"));
    let crawlers: Vec<String> = env::var("PDS_CRAWLERS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let trusted_clients: Vec<String> = env::var("PDS_OAUTH_TRUSTED_CLIENTS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let oauth_remote_rate_limit =
        env_or("PDS_RATELIMIT_OAUTH_REMOTE_PER_MINUTE", "50").parse().unwrap_or(50);

    let allowed_origins: HashSet<String> = env::var("PDS_CORS_ALLOWED_ORIGINS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    ServerConfig {
        service: ServiceConfig {
            hostname,
            public_url,
            port,
            service_did: env_or("PDS_SERVICE_DID", "did:web:localhost"),
        },
        service_db: ServiceDbConfig {
            account_db_location: env_or("PDS_ACCOUNT_DB_LOCATION", "./account.sqlite"),
            sequencer_db_location: env_or("PDS_SEQUENCER_DB_LOCATION", "./sequencer.sqlite"),
            did_cache_db_location: env_or("PDS_DID_CACHE_DB_LOCATION", "./did_cache.sqlite"),
        },
        identity: IdentityConfig {
            plc_url: env_or("PDS_PLC_URL", "https://plc.directory"),
            resolver_timeout: env_or("PDS_IDENTITY_RESOLVER_TIMEOUT_MS", "3000")
                .parse()
                .unwrap_or(3000),
            service_handle_domains: env::var("PDS_SERVICE_HANDLE_DOMAINS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            plc_client_mode: env_or("PDS_PLC_CLIENT_MODE", "http"),
        },
        actor_store: ActorStoreConfig {
            directory: env_or("PDS_ACTOR_STORE_DIRECTORY", "./actors"),
            cache_size: env_or("PDS_ACTOR_STORE_CACHE_SIZE", "100")
                .parse()
                .unwrap_or(100),
        },
        blobstore: BlobstoreConfig {
            disk_location: env_or("PDS_BLOBSTORE_DISK_LOCATION", "./blobs"),
        },
        cors: CorsConfig { allowed_origins },
        health: HealthConfig {
            version: env_or("PDS_VERSION", "0.0.0-test"),
        },
        oauth: OauthConfig {
            trusted_clients,
            rate_limit_per_minute: oauth_remote_rate_limit,
        },
        rate_limit: RateLimitConfig {
            create_session_per_minute: env_or("PDS_RATELIMIT_CREATE_SESSION_PER_MINUTE", "10")
                .parse()
                .unwrap_or(10),
            create_account_per_minute: env_or("PDS_RATELIMIT_CREATE_ACCOUNT_PER_MINUTE", "10")
                .parse()
                .unwrap_or(10),
            password_reset_per_minute: env_or("PDS_RATELIMIT_PASSWORD_RESET_PER_MINUTE", "5")
                .parse()
                .unwrap_or(5),
            email_ops_per_minute: env_or("PDS_RATELIMIT_EMAIL_OPS_PER_MINUTE", "5")
                .parse()
                .unwrap_or(5),
        },
        crawlers,
    }
}

/// Per-actor storage configuration.
#[derive(Debug, Clone)]
pub struct ActorStoreConfig {
    pub directory: String,
    pub cache_size: usize,
}
