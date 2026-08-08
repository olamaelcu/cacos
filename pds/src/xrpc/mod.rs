//! XRPC route assembly.
//!
//! [`build_app`] returns the full poem [`poem::Endpoint`] tree, including
//! CORS, the request metrics middleware, `/metrics`, health endpoints,
//! the `/.well-known/atproto-did` handler, and every per-module route
//! builder. The OAuth route set (provider routes + headless-consent
//! remote API) is mounted conditionally when `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`
//! is set, exactly as Plan 07 wired it.

pub mod com;
pub mod error;
pub mod health;
pub mod metrics;
pub mod test_utils;
pub mod types;
pub mod well_known;

pub mod app;

pub use error::{ApiError, ApiResult, ErrorBody};

use crate::account::AccountManager;
use crate::actor_store::ActorStore;
use crate::blobstore::{BlobStore, BoxedBlobStream};
use crate::config::ServerConfig;
use crate::context::SharedSequencer;
use crate::plc::PlcClient;
use crate::sequencer::apalis_worker::SharedBroadcast;
use poem::http::Method;
use poem::middleware::Cors;
use poem::{EndpointExt, Route};
use rsky_identity::IdResolver;
use std::sync::Arc;
use tokio::sync::RwLock;

/// All components a handler may need. Registered once with `.data(state)`
/// and reached through poem's `Data<&SharedState>` extractor. The
/// `sequencer` and `shared_broadcast` fields are also registered by their
/// own types so Plan 07's `subscribe_repos` (which extracts
/// `Data<&SharedSequencer>` and `Data<&SharedBroadcast>`) resolves them.
#[derive(Clone)]
pub struct SharedState {
    pub account_manager: AccountManager,
    pub actor_store: Arc<ActorStore>,
    pub sequencer: SharedSequencer,
    pub shared_broadcast: SharedBroadcast,
    pub id_resolver: Arc<RwLock<IdResolver>>,
    pub config: ServerConfig,
    pub plc_client: Arc<dyn PlcClient>,
    pub blobstore: Arc<dyn BlobStore<Stream = BoxedBlobStream>>,
}

/// Builds the full application from a pre-assembled [`SharedState`].
/// `main.rs` (and downstream plans) call this form. The OAuth route set
/// is mounted when the JWT key env is set; otherwise only the metrics +
/// XRPC + well_known + health routes are wired.
pub async fn build_app_with_state(
    state: SharedState,
) -> impl poem::Endpoint<Output = poem::Response> {
    let cors = Cors::new()
        .allow_origins_fn(|_| true)
        .allow_methods([
            Method::POST,
            Method::GET,
            Method::PATCH,
            Method::OPTIONS,
            Method::DELETE,
        ])
        .allow_credentials(true)
        .max_age(3600);

    // XRPC tree: every module's `routes()` registers its full NSID paths
    // directly (the XRPC convention uses dots, not slashes, in the URL).
    // Each module adds handlers onto a single shared Route so the radix
    // tree has exactly one `/xrpc/*` catch-all sibling; exact NSIDs win,
    // unmatched paths get the XRPC `{error,message}` shape.
    let mut xrpc_routes = Route::new();
    xrpc_routes = com::atproto::sync::routes(xrpc_routes);
    xrpc_routes = com::atproto::server::routes(xrpc_routes);
    xrpc_routes = com::atproto::repo::routes(xrpc_routes);
    xrpc_routes = com::atproto::identity::routes(xrpc_routes);
    xrpc_routes = com::atproto::admin::routes(xrpc_routes);
    xrpc_routes = com::atproto::temp::routes(xrpc_routes);
    xrpc_routes = app::bsky::actor::routes(xrpc_routes);
    xrpc_routes = xrpc_routes
        .at("/_health", poem::get(health::health))
        .at("/_health/live", poem::get(health::health_live))
        .at("/*rest", poem::get(unknown_xrpc_route));

    let mut base = Route::new()
        .nest("/xrpc", xrpc_routes)
        .at(
            "/.well-known/atproto-did",
            poem::get(well_known::well_known),
        );
    // Mount `/metrics` directly via `.at(...)` (not via `.nest(...)`) so
    // the radix tree doesn't pick up a second `*--poem-rest` catch-all
    // entry that would conflict with the OAuth nest below.
    base = base.at("/metrics", poem::get(crate::observability::http::metrics_handler));

    if std::env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX").is_ok() {
        let db_path =
            std::env::var("PDS_DB_PATH").unwrap_or_else(|_| "./account.sqlite".to_string());
        let oauth_app = match crate::db::DatabaseKind::Account
            .open(camino::Utf8Path::new(&db_path))
            .await
        {
            Ok(account_db) => crate::oauth::bootstrap_oauth_app(account_db),
            Err(err) => {
                tracing::error!(%err, "failed to open account database for OAuth");
                None
            }
        };
        if let Some(app) = oauth_app {
            base = base.nest_no_strip("/", app);
        }
    }

    base.data(state.clone())
        .data(state.sequencer.clone())
        .data(state.shared_broadcast.clone())
        .with(metrics::RequestMetrics)
        .with(cors)
}

/// Builds the app from environment-derived state. Convenience wrapper
/// used by `main.rs` and existing integration tests.
pub async fn build_app() -> impl poem::Endpoint<Output = poem::Response> {
    let cfg = crate::config::env_to_cfg();
    let state = types::SharedStateFromEnv::from_env(&cfg).await;
    build_app_with_state(state).await
}

/// Catch-all for unmatched /xrpc paths, rendering the XRPC
/// `{error, message}` JSON shape. NOTE: the reference's Rocket
/// `#[catch(default)]` returns 500 (`ApiError::RuntimeError`) for
/// unmatched routes; this handler pins the spec's 404 status with the
/// same body for client-friendly behavior.
#[poem::handler]
async fn unknown_xrpc_route() -> poem::Response {
    poem::Response::builder()
        .status(poem::http::StatusCode::NOT_FOUND)
        .content_type("application/json")
        .body(r#"{"error":"InternalServerError","message":"Something went wrong"}"#)
}
