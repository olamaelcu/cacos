# Task 1: XRPC Foundation — `ApiError`, `build_app()`, metrics middleware, test utils

**Files:**
- Create: `pds/src/xrpc/error.rs`
- Create: `pds/src/xrpc/mod.rs`
- Create: `pds/src/xrpc/metrics.rs`
- Create: `pds/src/xrpc/test_utils.rs`
- Modify: `pds/src/main.rs` (call `build_app`)

- [ ] **Step 1: Write the failing test for the health endpoint + error shape**

Create `pds/src/xrpc/mod.rs` with a placeholder `build_app` that has no routes yet, and `pds/src/xrpc/error.rs` empty. Test:

```rust
// pds/tests/xrpc_smoke.rs
use poem::test::TestClient;
use poem::Route;
use tempfile::tempdir;

// Stand-in: Task 1 wires the real app; this test only pins the route shape we
// will build in Task 1's Step 3. It is replaced by the real smoke test below.
#[tokio::test]
async fn placeholder() {
    let app = Route::new();
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/_health").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
}
```

Run: `cargo test -p pds --test xrpc_smoke placeholder`
Expected: PASS (route tree is empty → 404).

- [ ] **Step 2: Write the failing real smoke tests**

Replace the placeholder test with:

```rust
// pds/tests/xrpc_smoke.rs
use pds::xrpc::build_app;
use poem::test::TestClient;

#[tokio::test]
async fn health_returns_version() {
    let (state, _dirs) = pds::xrpc::test_utils::test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/_health").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["version"], "0.0.0-test");
}

#[tokio::test]
async fn unknown_route_returns_xrpc_error_shape() {
    let (state, _dirs) = pds::xrpc::test_utils::test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/com.atproto.nope.missing").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InternalServerError");
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn cors_headers_present() {
    let (state, _dirs) = pds::xrpc::test_utils::test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/_health/live").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert_eq!(
        resp.0.content_type().unwrap_or_default(),
        "text/plain; charset=utf-8"
    );
    // live probe must not touch the DB
    let body = resp.0.into_body().into_string().await.unwrap();
    assert_eq!(body, "ok");
}
```

Run: `cargo test -p pds --test xrpc_smoke`
Expected: FAIL — `build_app` / `test_state` do not exist yet.

- [ ] **Step 3: Implement `xrpc/error.rs` — the `ApiError` enum and its poem `IntoResponse`**

Port the variant set from `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/mod.rs` (lines 116–141) exactly; adapt the Rocket `Responder` (lines 149–478) into a poem `IntoResponse`. Statuses: RuntimeError→500, InvalidLogin/AccountTakendown/InvalidRequest/ExpiredToken/InvalidToken/InvalidHandle/InvalidEmail/InvalidPassword/InvalidInviteCode/HandleNotAvailable/EmailNotAvailable/UnsupportedDomain/UnresolvableDid/IncompatibleDidDoc/AccountNotFound/BlobNotFound/BadRequest→400, WellKnownNotFound/RecordNotFound→404, AuthRequiredError→401, UpstreamResponse→(status, error, message).

```rust
// pds/src/xrpc/error.rs
use poem::http::StatusCode;
use poem::Response;
use poem::IntoResponse;
use serde::Serialize;

#[derive(Clone, Debug)]
pub enum ApiError {
    RuntimeError,
    InvalidLogin,
    AccountTakendown,
    InvalidRequest(String),
    ExpiredToken,
    InvalidToken,
    RecordNotFound,
    InvalidHandle,
    InvalidEmail,
    InvalidPassword,
    InvalidInviteCode,
    HandleNotAvailable,
    EmailNotAvailable,
    UnsupportedDomain,
    UnresolvableDid,
    IncompatibleDidDoc,
    WellKnownNotFound,
    AccountNotFound,
    BlobNotFound,
    BadRequest(String, String),
    AuthRequiredError(String),
    /// Error passed through from an upstream service: status code, error, message
    UpstreamResponse(u16, String, String),
}

#[derive(Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ApiError {
    fn body(self) -> (StatusCode, ErrorBody) {
        match self {
            ApiError::RuntimeError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    error: "InternalServerError".to_string(),
                    message: "Something went wrong".to_string(),
                },
            ),
            ApiError::InvalidLogin => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidLogin".to_string(),
                    message: "Invalid identifier or password".to_string(),
                },
            ),
            ApiError::AccountTakendown => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "AccountTakendown".to_string(),
                    message: "Account has been taken down".to_string(),
                },
            ),
            ApiError::InvalidRequest(message) => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidRequest".to_string(),
                    message,
                },
            ),
            ApiError::ExpiredToken => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "ExpiredToken".to_string(),
                    message: "Token is expired".to_string(),
                },
            ),
            ApiError::InvalidToken => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidToken".to_string(),
                    message: "Token is invalid".to_string(),
                },
            ),
            ApiError::InvalidHandle => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidHandle".to_string(),
                    message: "Handle is invalid".to_string(),
                },
            ),
            ApiError::InvalidEmail => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidEmail".to_string(),
                    message: "Invalid email".to_string(),
                },
            ),
            ApiError::InvalidPassword => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidPassword".to_string(),
                    message: "Invalid Password".to_string(),
                },
            ),
            ApiError::InvalidInviteCode => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidInviteCode".to_string(),
                    message: "Invalid invite code".to_string(),
                },
            ),
            ApiError::HandleNotAvailable => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "HandleNotAvailable".to_string(),
                    message: "Handle not available".to_string(),
                },
            ),
            ApiError::EmailNotAvailable => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "EmailNotAvailable".to_string(),
                    message: "Email not available".to_string(),
                },
            ),
            ApiError::UnsupportedDomain => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "UnsupportedDomain".to_string(),
                    message: "Unsupported domain".to_string(),
                },
            ),
            ApiError::UnresolvableDid => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "UnresolvableDid".to_string(),
                    message: "Unresolved Did".to_string(),
                },
            ),
            ApiError::IncompatibleDidDoc => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "IncompatibleDidDoc".to_string(),
                    message: "IncompatibleDidDoc".to_string(),
                },
            ),
            ApiError::AccountNotFound => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "AccountNotFound".to_string(),
                    message: "Account could not be found".to_string(),
                },
            ),
            ApiError::BlobNotFound => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "BlobNotFound".to_string(),
                    message: "Blob could not be found".to_string(),
                },
            ),
            ApiError::WellKnownNotFound => (
                StatusCode::NOT_FOUND,
                ErrorBody {
                    error: "WellKnownNotFound".to_string(),
                    message: "User not found".to_string(),
                },
            ),
            ApiError::BadRequest(error, message) => {
                (StatusCode::BAD_REQUEST, ErrorBody { error, message })
            }
            ApiError::AuthRequiredError(message) => (
                StatusCode::UNAUTHORIZED,
                ErrorBody {
                    error: "AuthRequiredError".to_string(),
                    message,
                },
            ),
            ApiError::UpstreamResponse(status, error, message) => (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                ErrorBody { error, message },
            ),
            ApiError::RecordNotFound => (
                StatusCode::NOT_FOUND,
                ErrorBody {
                    error: "RecordNotFound".to_string(),
                    message: "Record could not be found".to_string(),
                },
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = self.body();
        let bytes = match serde_json::to_vec(&body) {
            Ok(bytes) => bytes,
            Err(_) => br#"{"error":"InternalServerError","message":"Something went wrong"}"#.to_vec(),
        };
        Response::builder()
            .status(status)
            .content_type("application/json")
            .body(bytes)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(_value: anyhow::Error) -> Self {
        ApiError::RuntimeError
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
```

- [ ] **Step 4: Implement `xrpc/metrics.rs` — per-NSID request metrics middleware**

```rust
// pds/src/xrpc/metrics.rs
use poem::Endpoint;
use poem::IntoResponse;
use poem::Middleware;
use poem::Request;
use poem::Response;
use poem::Result;
use std::time::Instant;

/// Metrics from Plan 02 use the `cacos_` prefix.
pub const REQUESTS_TOTAL: &str = "cacos_http_requests_total";
pub const REQUEST_ERRORS_TOTAL: &str = "cacos_http_request_errors_total";
pub const REQUEST_DURATION_SECONDS: &str = "cacos_http_request_duration_seconds";

pub struct RequestMetrics;

impl<E: Endpoint> Middleware<E> for RequestMetrics {
    type Output = RequestMetricsEndpoint<E>;

    fn transform(&self, ep: E) -> Self::Output {
        RequestMetricsEndpoint { ep }
    }
}

pub struct RequestMetricsEndpoint<E> {
    ep: E,
}

impl<E: Endpoint> Endpoint for RequestMetricsEndpoint<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> Result<Self::Output> {
        let start = Instant::now();
        let nsid = req.uri().path().to_string();
        let resp = self.ep.call(req).await;
        let is_error = match &resp {
            Ok(r) => !r.status().is_success(),
            Err(_) => true,
        };
        if is_error {
            metrics::increment_counter!(REQUEST_ERRORS_TOTAL, "nsid" => nsid.clone());
        }
        metrics::histogram!(
            REQUEST_DURATION_SECONDS,
            start.elapsed().as_secs_f64(),
            "nsid" => nsid.clone()
        );
        metrics::increment_counter!(REQUESTS_TOTAL, "nsid" => nsid);
        resp
    }
}
```

- [ ] **Step 5: Implement `xrpc/mod.rs` — `SharedState`, `build_app()`, health/well_known mounts, placeholder module mounts**

`build_app(state)` returns a `poem::Route` with the CORS layer, the metrics middleware, `/metrics` (Plan 02), health, well_known, and per-module route builders. Each handler module gets its own `pub fn routes() -> Route` (placeholders now, filled in by Tasks 5–26).

```rust
// pds/src/xrpc/mod.rs
pub mod auth_extractors;
pub mod com;
pub mod error;
pub mod health;
pub mod metrics;
pub mod test_utils;
pub mod types;
pub mod well_known;

pub use error::{ApiError, ApiResult, ErrorBody};

use crate::account::AccountManager;
use crate::actor_store::ActorStore;
use crate::blobstore::BlobStore;
use crate::config::ServerConfig;
use crate::context::{SharedBroadcast, SharedSequencer};
use crate::plc::PlcClient;
use poem::middleware::Cors;
use poem::http::Method;
use poem::{get, EndpointExt, Route};
use rsky_identity::IdResolver;
use std::sync::Arc;
use tokio::sync::RwLock;

/// All components a handler may need. Registered once with `.data(state)` and
/// reached through poem's `State<SharedState>` extractor. The `sequencer` and
/// `shared_broadcast` fields are also registered by their own types so
/// Plan 07's `subscribe_repos` (which extracts `State<SharedSequencer>` and
/// `State<SharedBroadcast>`) resolves them.
#[derive(Clone)]
pub struct SharedState {
    pub account_manager: AccountManager,
    pub actor_store: ActorStore,
    pub sequencer: SharedSequencer,
    pub shared_broadcast: SharedBroadcast,
    pub id_resolver: Arc<RwLock<IdResolver>>,
    pub config: ServerConfig,
    pub plc_client: Arc<dyn PlcClient>,
    pub blobstore: Arc<dyn BlobStore>,
}

/// Builds the full poem application. `main.rs` calls this with the assembled
/// `SharedState`.
pub fn build_app(state: SharedState) -> Route {
    let cors = Cors::new()
        .allow_origin_fn(|_| true)
        .allow_methods(vec![
            Method::POST,
            Method::GET,
            Method::PATCH,
            Method::OPTIONS,
            Method::DELETE,
        ])
        .allow_headers(vec!["*"])
        .allow_credentials(true)
        .max_age(3600);

    let health = Route::new()
        .at("/_health", get(health::health))
        .at("/_health/live", get(health::health_live));

    let xrpc = Route::new()
        .nest("/com.atproto.server", com::atproto::server::routes())
        .nest("/com.atproto.repo", com::atproto::repo::routes())
        .nest("/com.atproto.identity", com::atproto::identity::routes())
        .nest("/com.atproto.sync", com::atproto::sync::routes())
        .nest("/com.atproto.admin", com::atproto::admin::routes())
        .nest("/com.atproto.temp", com::atproto::temp::routes())
        .nest("/app.bsky.actor", crate::xrpc::app::bsky::actor::routes());

    Route::new()
        .at("/.well-known/atproto-did", get(well_known::well_known))
        .nest("/xrpc", xrpc)
        .nest("/xrpc", health)
        .nest("", metrics::metrics_routes())
        .at("/xrpc/*rest", get(unknown_xrpc_route))
        .data(state.clone())
        .data(state.sequencer.clone())
        .data(state.shared_broadcast.clone())
        .with(RequestMetrics)
        .with(cors)
}

/// Catch-all for unmatched /xrpc paths, rendering the XRPC `{error, message}`
/// JSON shape. NOTE: the reference's Rocket `#[catch(default)]` returns 500
/// (`ApiError::RuntimeError`) for unmatched routes; this plan pins the spec's
/// requested 404 status with the same body. For strict reference parity,
/// change `NOT_FOUND` to `INTERNAL_SERVER_ERROR` here and in the smoke test.
#[poem::handler]
async fn unknown_xrpc_route() -> poem::Response {
    poem::Response::builder()
        .status(poem::http::StatusCode::NOT_FOUND)
        .content_type("application/json")
        .body(r#"{"error":"InternalServerError","message":"Something went wrong"}"#)
}
```

Note the `health` endpoints are mounted under `/xrpc` via a second `.nest("/xrpc", health)` (poem nests multiple routes under the same prefix cleanly).

- [ ] **Step 6: Implement `xrpc/health.rs`**

```rust
// pds/src/xrpc/health.rs
use crate::xrpc::{ApiResult, SharedState};
use poem::http::StatusCode;
use poem::Response;
use poem::State;
use serde::Serialize;

#[derive(Serialize)]
pub struct ServerVersion {
    pub version: String,
}

/// `/xrpc/_health`: touches the account DB; 503 with a JSON body on failure.
#[poem::handler]
pub async fn health(state: State<SharedState>) -> ApiResult<Response> {
    let ok = state
        .account_manager
        .db
        .run(|conn| Ok(conn.query_row("SELECT 1", [], |row| row.get::<_, i32>(0))?))
        .await;
    match ok {
        Ok(_) => {
            let version = std::env::var("PDS_VERSION").unwrap_or_else(|_| "0.3.0-beta.3".into());
            Ok(Response::builder()
                .status(StatusCode::OK)
                .content_type("application/json")
                .body(serde_json::to_vec(&ServerVersion { version }).unwrap()))
        }
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Ok(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .content_type("application/json")
                .body(
                    serde_json::json!({
                        "error": "ServiceUnavailable",
                        "message": error.to_string(),
                    })
                    .to_string(),
                ))
        }
    }
}

/// `/xrpc/_health/live`: pure liveness probe, never touches the DB.
#[poem::handler]
pub async fn health_live() -> &'static str {
    "ok"
}
```

Note: `AccountManager::db` and `Db::run(|conn| ...)` are assumed from Plan 05 (the reference `AccountManager` has `pub db: Db` with `run`). Adjust the field path if Plan 05 named it differently.

- [ ] **Step 7: Mount the Plan 02 metrics route + `xrpc/types.rs` + placeholder module files**

The `/metrics` route belongs to Plan 02: `crate::observability::http::metrics_route()` returns a poem `Route` serving `GET /metrics` (Prometheus text). Mount it directly; do not re-implement the recorder.

```rust
// pds/src/xrpc/metrics.rs (append)
use poem::Route;

/// The /metrics route is owned by Plan 02 (observability::http::metrics_route).
/// This module only contributes the request-middleware metrics (Task 1 Step 4).
pub fn metrics_routes() -> Route {
    crate::observability::http::metrics_route()
}
```

`xrpc/types.rs` — small shared structs used across handlers (ports of rsky's lib.rs shared wrappers):

```rust
// pds/src/xrpc/types.rs
use rsky_identity::IdResolver;
use std::sync::Arc;
use tokio::sync::RwLock;

/// rsky wraps IdResolver in a RwLock and passes it everywhere; same here.
#[derive(Clone)]
pub struct SharedIdResolver {
    pub id_resolver: Arc<RwLock<IdResolver>>,
}
```

Placeholder module files (each with an empty `routes()` that later tasks fill in):

```rust
// pds/src/xrpc/com/mod.rs
pub mod atproto;
// pds/src/xrpc/com/atproto/mod.rs
pub mod admin;
pub mod identity;
pub mod repo;
pub mod server;
pub mod sync;
pub mod temp;
// pds/src/xrpc/app/mod.rs
pub mod bsky;
// pds/src/xrpc/app/bsky/mod.rs
pub mod actor;
// pds/src/xrpc/com/atproto/server/mod.rs
use poem::Route;
pub fn routes() -> Route {
    Route::new()
}
// pds/src/xrpc/com/atproto/repo/mod.rs
use poem::Route;
pub fn routes() -> Route {
    Route::new()
}
// pds/src/xrpc/com/atproto/identity/mod.rs
use poem::Route;
pub fn routes() -> Route {
    Route::new()
}
// pds/src/xrpc/com/atproto/sync/mod.rs
use poem::Route;
pub fn routes() -> Route {
    Route::new()
}
// pds/src/xrpc/com/atproto/admin/mod.rs
use poem::Route;
pub fn routes() -> Route {
    Route::new()
}
// pds/src/xrpc/com/atproto/temp/mod.rs
use poem::Route;
pub fn routes() -> Route {
    Route::new()
}
// pds/src/xrpc/app/bsky/actor/mod.rs
use poem::Route;
pub fn routes() -> Route {
    Route::new()
}
```

- [ ] **Step 8: Implement `xrpc/test_utils.rs` — env setup + temp `SharedState` + account/JWT helpers**

Tests throughout this plan use these helpers. The constructors come from Plans 01–07; the env defaults mirror `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/account_manager/tests.rs` (`init_env`).

```rust
// pds/src/xrpc/test_utils.rs
use crate::account::AccountManager;
use crate::actor_store::ActorStore;
use crate::blobstore::OpenDALBlobStore;
use crate::config::ServerConfig;
use crate::context::{SharedBroadcast, SharedSequencer};
use crate::plc::{MockPlcClient, PlcClient};
use crate::sequencer::Sequencer;
use crate::xrpc::SharedState;
use rsky_identity::IdResolver;
use std::sync::Arc;
use std::sync::Once;
use tokio::sync::RwLock;

static INIT_ENV: Once = Once::new();

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
                std::env::set_var(key, value);
            }
        }
    });
}

/// Builds a `SharedState` backed by temp directories. Returns the state plus
/// the temp dirs (kept alive so SQLite files are not deleted mid-test).
pub async fn test_state() -> (SharedState, Vec<tempfile::TempDir>) {
    init_env();
    let dir = tempfile::tempdir().unwrap();
    let actor_dir = tempfile::tempdir().unwrap();
    let account_db = crate::db::get_migrated_db(dir.path().join("account.sqlite"))
        .await
        .unwrap();
    // Plan 07 reconciliation:
    //   - Open via `DatabaseKind::Sequencer.open(path)` (impl
    //     `pds/src/db/mod.rs:39-66`). `crate::db::get_migrated_db` is the old
    //     name; the plan drops it.
    //   - `Sequencer::new` takes 4 args: `(db, crawlers, job_storage,
    //     last_seen)`. The 1-arg form below is replaced by the 4-arg form
    //     once the executor lands the rest of Plan 07. The placeholder here
    //     keeps `test_state()` compiling in isolation.
    let sequencer_db = crate::db::DatabaseKind::Sequencer
        .open(camino::Utf8Path::new(&dir.path().join("sequencer.sqlite")))
        .await
        .unwrap();
    let _ = sequencer_db;
    let account_manager = AccountManager::new(account_db);
    let sequencer = SharedSequencer {
        sequencer: RwLock::new(/* TODO Plan 07 4-arg ctor */ unimplemented!()),
    };
    let shared_broadcast = SharedBroadcast {
        tx: tokio::sync::broadcast::channel(1024).0,
    };
    let config = crate::config::env_to_cfg();
    let actor_store = ActorStore::new(&config.actor_store);
    // Plan 06: account-status checks inside validate_access_token need the
    // registered AccountManager.
    crate::auth::auth_verifier::register_auth_dependencies(
        std::sync::Arc::new(account_manager.clone()),
        None,
    );
    let state = SharedState {
        account_manager,
        actor_store,
        sequencer,
        shared_broadcast,
        id_resolver: Arc::new(RwLock::new(IdResolver::new(
            rsky_identity::types::IdentityResolverOpts::default(),
        ))),
        config,
        plc_client: Arc::new(MockPlcClient::default()),
        blobstore: Arc::new(OpenDALBlobStore::new(&config.blobstore, &actor_dir.path())),
    };
    let _ = actor_dir;
    (state, vec![dir])
}

/// Creates an account directly through the AccountManager and returns its
/// (access_jwt, refresh_jwt) — the same tokens createSession would issue.
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
            repo_cid: "bafkreibjfgx2gprinfvicegelk5kosd6y2frmqpqzwqkg7usac74l3t2v4".parse().unwrap(),
            repo_rev: "3jzfcijpj2z2a".to_owned(),
            invite_code: None,
            deactivated: None,
        })
        .await
        .unwrap()
}
```

> **Adaptation note:** the exact constructor signatures above (`Db::get_migrated_db`, `Sequencer::new`, `ActorStore::new`, `OpenDALBlobStore::new`, `IdResolver::new`, `CreateAccountOpts` fields) must match Plans 01–07. If a constructor differs, adjust only `test_utils.rs` — handler code below never constructs these directly. `PlcClient` and `MockPlcClient` must exist before this compiles: create a minimal `pds/src/plc/mod.rs` now with the `PlcClient` trait (four async methods: `send_operation`, `get_document_data`, `get_last_op`, `update_handle`) and a `MockPlcClient` with a `Default` impl returning `Ok(())` from every method. Task 18 replaces this scaffold with the full trait + reqwest `PlcClientImpl` + the richer mock. The Task 1 minimal mock needs only the default constructor for `test_utils`; tests that require realistic PLC data start at Task 7.

- [ ] **Step 9: Wire `main.rs`**

```rust
// pds/src/main.rs (assumes Plans 01–07 assembled the components)
mod account;
mod actor_store;
mod auth;
mod blobstore;
mod config;
mod context;
mod db;
mod identity;
mod mailer;
mod observability;
mod oauth;
mod plc;
mod sequencer;
mod xrpc;

use crate::xrpc::build_app;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    observability::tracing::init();
    let cfg = config::env_to_cfg();
    let state = crate::xrpc::types::SharedStateFromEnv::from_env(&cfg).await;
    let app = build_app(state);
    poem::Server::new(poem::listener::TcpListener::bind(format!("0.0.0.0:{}", cfg.service.port)))
        .run(app)
        .await
}
```

`SharedStateFromEnv::from_env` is a thin constructor assembled in `xrpc/types.rs` (Task 1, Step 10). If the exact module list of Plans 01–07 differs, reconcile `mod` declarations with what exists.

- [ ] **Step 10: Add `SharedStateFromEnv` to `xrpc/types.rs`**

```rust
// append to pds/src/xrpc/types.rs
use crate::config::ServerConfig;
use crate::xrpc::SharedState;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SharedStateFromEnv;

impl SharedStateFromEnv {
    /// Assembles the full runtime state from a loaded ServerConfig. Each
    /// constructor below comes from Plans 01–07.
    pub async fn from_env(cfg: &ServerConfig) -> SharedState {
        let account_db = crate::db::get_migrated_db(&cfg.service_db.account_db_location)
            .await
            .expect("failed to open account database");
        // Plan 07 reconciliation: open via `DatabaseKind::Sequencer.open(path)`.
        let sequencer_db = crate::db::DatabaseKind::Sequencer
            .open(camino::Utf8Path::new(&cfg.service_db.sequencer_db_location))
            .await
            .expect("failed to open sequencer database");
        let did_cache_db = crate::db::get_migrated_db(&cfg.service_db.did_cache_db_location)
            .await
            .expect("failed to open did cache database");

        let account_manager = crate::account::AccountManager::new(account_db);
        // Plan 07 reconciliation: 4-arg `Sequencer::new(db, crawlers,
        // job_storage, last_seen)`. The apalis jobs DB is wired here; the
        // `Crawlers` reads the PDS hostname from `cfg.service.hostname`.
        let sequencer = crate::context::SharedSequencer {
            sequencer: RwLock::new(crate::sequencer::Sequencer::new(
                sequencer_db,
                crate::sequencer::crawlers::Crawlers::new(
                    cfg.service.hostname.clone(),
                    cfg.crawlers.clone(),
                ),
                /* TODO Plan 07 Task 4: `apalis_worker::connect_jobs_db(path)` */
                unimplemented!(),
                None,
            )),
        };
        let shared_broadcast = crate::context::SharedBroadcast {
            tx: tokio::sync::broadcast::channel(1024).0,
        };
        let id_resolver = rsky_identity::IdResolver::new(rsky_identity::types::IdentityResolverOpts {
            timeout: Some(std::time::Duration::from_millis(cfg.identity.resolver_timeout)),
            plc_url: Some(cfg.identity.plc_url.clone()),
            did_cache: Some(Arc::new(crate::identity::DidSqliteCache::new(
                did_cache_db,
            ))),
            ..Default::default()
        });
        let actor_store = crate::actor_store::ActorStore::new(&cfg.actor_store);
        let blobstore: Arc<dyn crate::blobstore::BlobStore> =
            Arc::new(crate::blobstore::OpenDALBlobStore::new(
                &cfg.blobstore,
                std::path::Path::new(&cfg.actor_store.directory),
            ));
        let plc_client: Arc<dyn crate::plc::PlcClient> =
            Arc::new(crate::plc::PlcClientImpl::new(cfg.identity.plc_url.clone()));

        // Plan 06 auth dependencies: the pure verifier reads the AccountManager
        // (account-status checks) and optional OAuth provider (DPoP) from
        // statics registered here. register_signing_key_resolver is wired once
        // IdResolver exists (Plan 06 Task 4).
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
```

- [ ] **Step 11: Run the tests to verify they pass**

Run: `cargo test -p pds --test xrpc_smoke`
Expected: PASS (3 tests: health_returns_version, unknown_route_returns_xrpc_error_shape, cors_headers_present).

Also run: `cargo check -p pds`
Expected: compiles with no errors (placeholder module warnings are fine).

- [ ] **Step 12: Commit**

```bash
git add pds/src/xrpc pds/src/main.rs pds/tests/xrpc_smoke.rs
git commit -m "feat(xrpc): ApiError mapping, build_app with CORS+metrics, health endpoints, test utils"
```
