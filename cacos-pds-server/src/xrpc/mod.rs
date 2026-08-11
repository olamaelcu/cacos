//! XRPC route assembly.
//!
//! [`build_app`] returns the full poem [`poem::Endpoint`] tree, including
//! CORS, the request metrics middleware, `/metrics`, health endpoints,
//! the `/.well-known/atproto-did` handler, and every per-module route
//! builder. The OAuth route set (provider routes + headless-consent
//! remote API) is mounted conditionally when `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`
//! is set, exactly as Plan 07 wired it.

pub mod app;
pub mod auth_extractors;
pub mod com;
pub mod cors;
pub mod error;
pub mod health;
pub mod metrics;
// `rate_limit` moved to cacos-pds-oauth.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
pub mod types;
pub mod well_known;

pub use error::{ApiError, ApiResult, ErrorBody};

use cacos_pds_account::account::AccountManager;
use cacos_pds_actor_store::ActorStore;
use cacos_pds_blobstore::{BlobStore, BoxedBlobStream};
use cacos_pds_core::config::ServerConfig;
use cacos_pds_sequencer::shared_sequencer::SharedSequencer;
use cacos_pds_plc::PlcClient;
use cacos_pds_sequencer::apalis_worker::SharedBroadcast;
use cors::CorsPolicy;
use poem::http::Method;
use poem::middleware::{Cors, Middleware};
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
    let cors_policy = CorsPolicy::from_env(&state.config.service.public_url);
    let cors = Cors::new()
        .allow_origins_fn(move |origin| cors_policy.allows(Some(origin)))
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
    xrpc_routes = wire_server_routes_with_rate_limits(xrpc_routes);
    xrpc_routes = com::atproto::repo::routes(xrpc_routes);
    xrpc_routes = com::atproto::identity::routes(xrpc_routes);
    xrpc_routes = com::atproto::admin::routes(xrpc_routes);
    xrpc_routes = com::atproto::temp::routes(xrpc_routes);
    xrpc_routes = app::bsky::actor::routes(xrpc_routes);
    xrpc_routes = xrpc_routes
        .at("/_health", poem::get(health::health))
        .at("/_health/live", poem::get(health::health_live))
        .at("/*rest", poem::get(unknown_xrpc_route));

    let mut base = Route::new().nest("/xrpc", xrpc_routes).at(
        "/.well-known/atproto-did",
        poem::get(well_known::well_known),
    );
    // Mount `/metrics` directly via `.at(...)` (not via `.nest(...)`) so
    // the radix tree doesn't pick up a second `*--poem-rest` catch-all
    // entry that would conflict with the OAuth nest below.
    base = base.at(
        "/metrics",
        poem::get(cacos_pds_core::observability::http::metrics_handler),
    );

    if std::env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX").is_ok() {
        let db_path =
            std::env::var("PDS_DB_PATH").unwrap_or_else(|_| "./account.sqlite".to_string());
        let oauth_app = match cacos_pds_core::db::DatabaseKind::Account
            .open(camino::Utf8Path::new(&db_path))
            .await
        {
            Ok(account_db) => cacos_pds_oauth::bootstrap_oauth_app(
                account_db,
                state.account_manager.clone(),
                state.actor_store.clone(),
                state.plc_client.clone(),
                state.blobstore.clone(),
                state.sequencer.clone(),
            ),
            Err(err) => {
                tracing::error!(%err, "failed to open account database for OAuth");
                None
            }
        };
        let provider = oauth_app.as_ref().map(|b| Arc::clone(&b.provider));
        cacos_pds_account::auth::verifier::register_auth_dependencies(
            Arc::new(state.account_manager.clone()),
            provider,
        );
        if let Some(b) = oauth_app {
            base = base.nest_no_strip("/", b.endpoint);
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
    let cfg = cacos_pds_core::config::env_to_cfg();
    let state = types::SharedStateFromEnv::from_env(&cfg).await;
    build_app_with_state(state).await
}

fn env_limit(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(default)
}

fn wire_server_routes_with_rate_limits(route: poem::Route) -> poem::Route {
    use poem::{get, post};
    use cacos_pds_oauth::rate_limit::{RouteRateLimit, ip_limiter};

    // Defaults match the cacos-pds cacos-pds-OAuth security plan. A value of
    // 0 disables the per-route limiter.
    let create_session_limiter =
        ip_limiter(env_limit("PDS_RATELIMIT_CREATE_SESSION_PER_MINUTE", 10));
    let create_account_limiter =
        ip_limiter(env_limit("PDS_RATELIMIT_CREATE_ACCOUNT_PER_MINUTE", 10));
    let password_reset_limiter =
        ip_limiter(env_limit("PDS_RATELIMIT_PASSWORD_RESET_PER_MINUTE", 5));
    let email_ops_limiter = ip_limiter(env_limit("PDS_RATELIMIT_EMAIL_OPS_PER_MINUTE", 5));

    let rl = |l: std::sync::Arc<cacos_pds_oauth::rate_limit::IpRateLimiter>| RouteRateLimit { limiter: l };

    route
        .at(
            "/createSession",
            post(
                rl(create_session_limiter)
                    .transform(com::atproto::server::create_session::create_session),
            ),
        )
        .at(
            "/refreshSession",
            post(com::atproto::server::refresh_session::refresh_session),
        )
        .at(
            "/deleteSession",
            post(com::atproto::server::delete_session::delete_session),
        )
        .at(
            "/getSession",
            get(com::atproto::server::get_session::get_session),
        )
        .at(
            "/createAccount",
            post(
                rl(create_account_limiter)
                    .transform(com::atproto::server::create_account::server_create_account),
            ),
        )
        .at(
            "/activateAccount",
            post(com::atproto::server::activate_account::activate_account),
        )
        .at(
            "/deactivateAccount",
            post(com::atproto::server::deactivate_account::deactivate_account),
        )
        .at(
            "/deleteAccount",
            post(com::atproto::server::delete_account::delete_account),
        )
        .at(
            "/checkAccountStatus",
            get(com::atproto::server::check_account_status::check_account_status),
        )
        .at(
            "/confirmEmail",
            post(com::atproto::server::confirm_email::confirm_email),
        )
        .at(
            "/updateEmail",
            post(com::atproto::server::update_email::update_email),
        )
        .at(
            "/requestEmailConfirmation",
            post(rl(email_ops_limiter.clone()).transform(
                com::atproto::server::request_email_confirmation::request_email_confirmation,
            )),
        )
        .at(
            "/requestEmailUpdate",
            post(
                rl(email_ops_limiter)
                    .transform(com::atproto::server::request_email_update::request_email_update),
            ),
        )
        .at(
            "/requestPasswordReset",
            post(
                rl(password_reset_limiter.clone()).transform(
                    com::atproto::server::request_password_reset::request_password_reset,
                ),
            ),
        )
        .at(
            "/resetPassword",
            post(
                rl(password_reset_limiter)
                    .transform(com::atproto::server::reset_password::reset_password),
            ),
        )
        .at(
            "/requestAccountDelete",
            post(com::atproto::server::request_account_delete::request_account_delete),
        )
        .at(
            "/createAppPassword",
            post(com::atproto::server::create_app_password::create_app_password),
        )
        .at(
            "/listAppPasswords",
            get(com::atproto::server::list_app_passwords::list_app_passwords),
        )
        .at(
            "/revokeAppPassword",
            post(com::atproto::server::revoke_app_password::revoke_app_password),
        )
        .at(
            "/createInviteCode",
            post(com::atproto::server::create_invite_code::create_invite_code),
        )
        .at(
            "/createInviteCodes",
            post(com::atproto::server::create_invite_codes::create_invite_codes),
        )
        .at(
            "/getAccountInviteCodes",
            get(com::atproto::server::get_account_invite_codes::get_account_invite_codes),
        )
        .at(
            "/describeServer",
            get(com::atproto::server::describe_server::describe_server),
        )
        .at(
            "/getServiceAuth",
            get(com::atproto::server::get_service_auth::get_service_auth),
        )
        .at(
            "/reserveSigningKey",
            post(com::atproto::server::reserve_signing_key::reserve_signing_key),
        )
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
