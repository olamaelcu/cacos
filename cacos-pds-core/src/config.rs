//! Server configuration loaded from the environment.

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
/// and the bind port for the poem server.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub hostname: String,
    pub public_url: String,
    pub port: u16,
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

    ServerConfig {
        service: ServiceConfig {
            hostname,
            public_url,
            port,
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
        crawlers,
    }
}


/// Per-actor storage configuration.
#[derive(Debug, Clone)]
pub struct ActorStoreConfig {
    pub directory: String,
    pub cache_size: usize,
}
