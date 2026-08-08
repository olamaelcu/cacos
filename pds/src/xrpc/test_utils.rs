//! Test helpers for the XRPC layer.
//!
//! [`test_state`] builds a fully-wired [`SharedState`] backed by temp
//! directories. Callers must keep the returned temp dirs alive for the
//! lifetime of the state.

use crate::account::AccountManager;
use crate::actor_store::ActorStore;
use crate::blobstore::{BlobStore, BoxedBlobStream, OpenDALBlobStore};
use crate::config::ServerConfig;
use crate::context::SharedSequencer;
use crate::identity::did_cache::DidSqliteCache;
use crate::plc::MockPlcClient;
use crate::sequencer::crawlers::Crawlers;
use crate::sequencer::{Sequencer, apalis_worker::SharedBroadcast};
use crate::xrpc::SharedState;
use rsky_identity::IdResolver;
use std::sync::{Arc, Once};
use tokio::sync::RwLock;

static INIT_ENV: Once = Once::new();

fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().to_string()
}

pub fn init_env() {
    INIT_ENV.call_once(|| {
        let defaults = [
            ("PDS_SERVICE_DID", "did:web:localho.st"),
            (
                "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
                "9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd",
            ),
            (
                "PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                "71cfcf4882a6cff494c3d0affadd3858eb3a5838e7b5e15170e696a590a4fa01",
            ),
            (
                "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
                "e7763b0a2d1a4f8e9f9d2d6f3f9a5d0c6b2e4a8c1f7d3e9b5a0c2f4d6e8a0b1c",
            ),
            ("PDS_HOSTNAME", "localhost"),
            ("PDS_SERVICE_HANDLE_DOMAINS", ".test"),
            ("PDS_ADMIN_PASSWORD", "admin-password"),
        ];
        for (key, value) in defaults {
            if std::env::var(key).is_err() {
                // SAFETY: tests run sequentially within a process, no
                // concurrent env reads.
                unsafe {
                    std::env::set_var(key, value);
                }
            }
        }
    });
}

/// Builds a [`SharedState`] backed by temp directories. Returns the state
/// plus the temp dirs (kept alive so SQLite files are not deleted
/// mid-test).
pub async fn test_state() -> (SharedState, Vec<tempfile::TempDir>) {
    init_env();
    let dir = tempfile::tempdir().unwrap();
    let actor_dir = tempfile::tempdir().unwrap();
    let blob_dir = tempfile::tempdir().unwrap();
    let account_db = crate::db::DatabaseKind::Account
        .open(camino::Utf8Path::from_path(&dir.path().join("account.sqlite")).unwrap())
        .await
        .unwrap();
    let sequencer_db = crate::db::DatabaseKind::Sequencer
        .open(camino::Utf8Path::from_path(&dir.path().join("sequencer.sqlite")).unwrap())
        .await
        .unwrap();
    let did_cache_db = crate::db::DatabaseKind::DidCache
        .open(camino::Utf8Path::from_path(&dir.path().join("did_cache.sqlite")).unwrap())
        .await
        .unwrap();

    let account_manager = AccountManager::new(account_db);
    let sequencer = SharedSequencer::new(Sequencer::new(
        sequencer_db,
        Crawlers::new("pds.test".to_string(), vec![]),
        None,
        None,
    ));
    let shared_broadcast = SharedBroadcast::new(1024);

    let id_resolver = IdResolver::new(rsky_identity::types::IdentityResolverOpts {
        timeout: Some(std::time::Duration::from_millis(3000)),
        plc_url: Some("https://plc.test".to_string()),
        did_cache: Some(Arc::new(DidSqliteCache::new(
            did_cache_db,
            crate::background::BackgroundQueue::default(),
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(60 * 60 * 24),
        ))),
        backup_nameservers: None,
    });

    let config = ServerConfig {
        service: crate::config::ServiceConfig {
            hostname: "pds.test".to_string(),
            public_url: "https://pds.test".to_string(),
            port: 8080,
        },
        service_db: crate::config::ServiceDbConfig {
            account_db_location: path_str(&dir.path().join("account.sqlite")),
            sequencer_db_location: path_str(&dir.path().join("sequencer.sqlite")),
            did_cache_db_location: path_str(&dir.path().join("did_cache.sqlite")),
        },
        identity: crate::config::IdentityConfig {
            plc_url: "https://plc.test".to_string(),
            resolver_timeout: 3000,
        },
        actor_store: crate::actor_store::ActorStoreConfig {
            directory: path_str(actor_dir.path()),
            cache_size: 10,
        },
        blobstore: crate::config::BlobstoreConfig {
            disk_location: path_str(blob_dir.path()),
        },
        crawlers: vec![],
    };

    let actor_store = Arc::new(ActorStore::new(&config.actor_store));
    let blobstore: Arc<dyn BlobStore<Stream = BoxedBlobStream>> = Arc::new(
        OpenDALBlobStore::new_disk(blob_dir.path(), "shared")
            .expect("build blobstore"),
    );
    let plc_client: Arc<dyn crate::plc::PlcClient> = Arc::new(MockPlcClient);

    // Plan 06: account-status checks inside validate_access_token need
    // the registered AccountManager.
    crate::auth::auth_verifier::register_auth_dependencies(
        Arc::new(account_manager.clone()),
        None,
    );

    let state = SharedState {
        account_manager,
        actor_store,
        sequencer,
        shared_broadcast,
        id_resolver: Arc::new(RwLock::new(id_resolver)),
        config,
        plc_client,
        blobstore,
    };
    (state, vec![dir, actor_dir, blob_dir])
}

/// Creates an account directly through the AccountManager and returns its
/// (access_jwt, refresh_jwt) — the same tokens `createSession` would issue.
pub async fn create_test_account(
    state: &SharedState,
    did: &str,
    handle: &str,
) -> (String, String) {
    state
        .account_manager
        .create_account(crate::account::CreateAccountOpts {
            did: did.to_owned(),
            handle: handle.to_owned(),
            email: Some(format!("{handle}@example.com")),
            password: Some("password123".to_owned()),
            repo_cid: "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4"
                .parse()
                .unwrap(),
            repo_rev: "3jzfcijpj2z2a".to_owned(),
            invite_code: None,
            deactivated: None,
        })
        .await
        .unwrap()
}
