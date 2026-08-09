//! Shared XRPC types: [`SharedIdResolver`] and [`SharedStateFromEnv`].
//!
//! [`SharedStateFromEnv`] is the runtime assembler used by [`crate::xrpc::build_app`]
//! to wire every collaborator (account DB, sequencer, ID resolver,
//! blobstore, PLC client, auth verifier) into the per-process
//! [`crate::xrpc::SharedState`].

use crate::account::AccountManager;
use crate::actor_store::ActorStore;
use crate::blobstore::OpenDALBlobStore;
use crate::config::ServerConfig;
use crate::context::SharedSequencer;
use crate::identity::did_cache::DidSqliteCache;
use crate::plc::PlcClient;
use crate::sequencer::crawlers::Crawlers;
use crate::xrpc::SharedState;
use rsky_identity::IdResolver;
use std::sync::Arc;
use tokio::sync::RwLock;

/// rsky wraps `IdResolver` in a `RwLock` and passes it everywhere; same
/// here.
#[derive(Clone)]
pub struct SharedIdResolver {
    pub id_resolver: Arc<RwLock<IdResolver>>,
}

/// Assembles the full runtime state from a loaded [`ServerConfig`]. Each
/// constructor below comes from Plans 01–07.
pub struct SharedStateFromEnv;

impl SharedStateFromEnv {
    pub async fn from_env(cfg: &ServerConfig) -> SharedState {
        let account_db = crate::db::DatabaseKind::Account
            .open(camino::Utf8Path::new(&cfg.service_db.account_db_location))
            .await
            .expect("failed to open account database");
        let sequencer_db = crate::db::DatabaseKind::Sequencer
            .open(camino::Utf8Path::new(&cfg.service_db.sequencer_db_location))
            .await
            .expect("failed to open sequencer database");
        let did_cache_db = crate::db::DatabaseKind::DidCache
            .open(camino::Utf8Path::new(&cfg.service_db.did_cache_db_location))
            .await
            .expect("failed to open did cache database");

        let account_manager = AccountManager::new(account_db);
        let sequencer = SharedSequencer::new(crate::sequencer::Sequencer::new(
            sequencer_db,
            Crawlers::new(cfg.service.hostname.clone(), cfg.crawlers.clone()),
            None,
            None,
        ));
        let shared_broadcast = crate::sequencer::apalis_worker::SharedBroadcast::new(1024);

        let id_resolver = IdResolver::new(rsky_identity::types::IdentityResolverOpts {
            timeout: Some(std::time::Duration::from_millis(cfg.identity.resolver_timeout)),
            plc_url: Some(cfg.identity.plc_url.clone()),
            did_cache: Some(Arc::new(DidSqliteCache::new(
                did_cache_db,
                crate::background::BackgroundQueue::default(),
                std::time::Duration::from_secs(60),
                std::time::Duration::from_secs(60 * 60 * 24),
            ))),
            backup_nameservers: None,
        });
        let actor_store = Arc::new(ActorStore::new(&cfg.actor_store));
        let blobstore: Arc<
            dyn crate::blobstore::BlobStore<Stream = crate::blobstore::BoxedBlobStream>,
        > = Arc::new(
            OpenDALBlobStore::new_disk(
                std::path::Path::new(&cfg.blobstore.disk_location),
                "shared",
            )
            .expect("build shared blobstore"),
        );
        let plc_client: Arc<dyn PlcClient> = Arc::new(crate::plc::MockPlcClient::default());

        crate::auth::auth_verifier::register_auth_dependencies(
            Arc::new(account_manager.clone()),
            None,
        );

        SharedState {
            account_manager,
            actor_store,
            sequencer,
            shared_broadcast,
            id_resolver: Arc::new(RwLock::new(id_resolver)),
            config: cfg.clone(),
            plc_client,
            blobstore,
        }
    }
}
