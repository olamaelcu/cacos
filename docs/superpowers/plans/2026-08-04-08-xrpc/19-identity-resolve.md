# Task 19: Identity resolve — resolveDid, resolveHandle, resolveIdentity, refreshIdentity

**Files:**
- Create: `pds/src/xrpc/com/atproto/identity/mod.rs` (route table; replace Task 1 placeholder)
- Create: `pds/src/xrpc/com/atproto/identity/resolve_did.rs`
- Create: `pds/src/xrpc/com/atproto/identity/resolve_handle.rs`
- Create: `pds/src/xrpc/com/atproto/identity/resolve_identity.rs`
- Create: `pds/src/xrpc/com/atproto/identity/refresh_identity.rs`
- Test: `pds/tests/identity_resolve_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/identity_resolve_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;

#[tokio::test]
async fn resolve_handle_for_local_account() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=alice.test")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:alice");
}

#[tokio::test]
async fn resolve_identity_by_handle_returns_info() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.identity.resolveIdentity?identifier=bob.test")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:bob");
    assert_eq!(body["handle"], "bob.test");
    assert!(body["didDoc"].is_object());
}

#[tokio::test]
async fn resolve_did_unknown_returns_bad_request() {
    let (state, _dirs) = test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.identity.resolveDid?did=did:plc:doesnotexist")
        .send()
        .await;
    // resolution of an unknown did:plc hits the network in the real resolver;
    // with no resolver configured the handler must not panic and returns 400
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "DidNotFound");
}

#[tokio::test]
async fn refresh_identity_local_account() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:carol", "carol.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.identity.refreshIdentity")
        .body_json(&serde_json::json!({ "identifier": "carol.test" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:carol");
}
```

> **Test note:** local account handles resolve via `account_manager` first; remote resolution (network) is not exercised in tests. `resolve_did_unknown_returns_bad_request` asserts the 400 `DidNotFound` shape — the IdResolver in `test_state()` has no network backends, so unknown DIDs fail resolution and surface as `DidNotFound`.

Run: `cargo test -p pds --test identity_resolve_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `resolve_identity.rs` (shared helpers) + `resolve_did.rs`**

```rust
// pds/src/xrpc/com/atproto/identity/resolve_identity.rs
use crate::account::AccountManager;
use crate::xrpc::types::SharedIdResolver;
use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;
use rsky_common::get_handle;
use rsky_identity::types::DidDocument;
use rsky_lexicon::com::atproto::identity::IdentityInfo;
use rsky_syntax::handle::INVALID_HANDLE;

pub enum Identifier {
    Did(String),
    Handle(String),
}

pub fn classify_identifier(identifier: &str) -> Identifier {
    if identifier.starts_with("did:") {
        Identifier::Did(identifier.to_string())
    } else {
        Identifier::Handle(identifier.to_lowercase())
    }
}

pub fn handles_match(doc_handle: &str, handle: &str) -> bool {
    doc_handle.eq_ignore_ascii_case(handle)
}

pub async fn resolve_handle_to_did(
    handle: &String,
    id_resolver: &SharedIdResolver,
    account_manager: &AccountManager,
) -> Result<String, ApiError> {
    if let Ok(Some(user)) = account_manager.get_account(handle, None).await {
        return Ok(user.did);
    }
    let resolved = {
        let mut lock = id_resolver.id_resolver.write().await;
        lock.handle.resolve(handle).await
    };
    match resolved {
        Ok(Some(did)) => Ok(did),
        _ => Err(ApiError::BadRequest(
            "HandleNotFound".to_string(),
            format!("unable to resolve handle: {handle}"),
        )),
    }
}

pub async fn resolve_did_doc(
    did: &String,
    force_refresh: bool,
    id_resolver: &SharedIdResolver,
) -> Result<DidDocument, ApiError> {
    let doc = {
        let lock = id_resolver.id_resolver.write().await;
        lock.did.resolve(did.clone(), Some(force_refresh)).await
    };
    match doc {
        Ok(Some(doc)) => Ok(doc),
        _ => Err(ApiError::BadRequest(
            "DidNotFound".to_string(),
            format!("could not resolve DID: {did}"),
        )),
    }
}

pub async fn inner_resolve_identity(
    identifier: String,
    force_refresh: bool,
    id_resolver: &SharedIdResolver,
    account_manager: &AccountManager,
) -> Result<IdentityInfo, ApiError> {
    let (did, input_handle) = match classify_identifier(&identifier) {
        Identifier::Did(did) => (did, None),
        Identifier::Handle(handle) => {
            let did = resolve_handle_to_did(&handle, id_resolver, account_manager).await?;
            (did, Some(handle))
        }
    };
    let doc = resolve_did_doc(&did, force_refresh, id_resolver).await?;
    let doc_handle = get_handle(&doc);
    let handle = match (doc_handle, input_handle) {
        (Some(doc_handle), Some(input_handle)) if handles_match(&doc_handle, &input_handle) => {
            doc_handle
        }
        (Some(_), Some(_)) => INVALID_HANDLE.to_string(),
        (Some(doc_handle), None) => {
            match resolve_handle_to_did(&doc_handle.to_lowercase(), id_resolver, account_manager)
                .await
            {
                Ok(resolved_did) if resolved_did == did => doc_handle,
                _ => INVALID_HANDLE.to_string(),
            }
        }
        (None, _) => INVALID_HANDLE.to_string(),
    };
    let did_doc = serde_json::to_value(&doc).map_err(|error| {
        tracing::error!("{error}");
        ApiError::RuntimeError
    })?;
    Ok(IdentityInfo {
        did,
        handle,
        did_doc,
    })
}

/// GET /xrpc/com.atproto.identity.resolveIdentity?<identifier>
#[poem::handler]
pub async fn resolve_identity(
    poem::web::Query(query): poem::web::Query<ResolveIdentityQuery>,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<Json<IdentityInfo>> {
    let ResolveIdentityQuery { identifier } = query;
    let info = inner_resolve_identity(
        identifier,
        false,
        &state.id_resolver,
        &state.account_manager,
    )
    .await?;
    Ok(Json(info))
}

#[derive(serde::Deserialize)]
pub struct ResolveIdentityQuery {
    pub identifier: String,
}
```

```rust
// pds/src/xrpc/com/atproto/identity/resolve_did.rs
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use rsky_lexicon::com::atproto::identity::ResolveDidOutput;

/// GET /xrpc/com.atproto.identity.resolveDid?<did>
#[poem::handler]
pub async fn resolve_did(
    poem::web::Query(query): poem::web::Query<ResolveDidQuery>,
    state: poem::State<SharedState>,
) -> ApiResult<Json<ResolveDidOutput>> {
    let ResolveDidQuery { did } = query;
    let doc = {
        let lock = state.id_resolver.write().await;
        lock.did.resolve(did.clone(), None).await
    };
    match doc {
        Ok(Some(doc)) => match serde_json::to_value(&doc) {
            Ok(did_doc) => Ok(Json(ResolveDidOutput { did_doc })),
            Err(error) => {
                tracing::error!("{error}");
                Err(ApiError::RuntimeError)
            }
        },
        Ok(None) => Err(ApiError::BadRequest(
            "DidNotFound".to_string(),
            format!("could not resolve DID: {did}"),
        )),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::BadRequest(
                "DidNotFound".to_string(),
                format!("could not resolve DID: {did}"),
            ))
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ResolveDidQuery {
    pub did: String,
}
```

- [ ] **Step 3: Implement `resolve_handle.rs` and `refresh_identity.rs`**

```rust
// pds/src/xrpc/com/atproto/identity/resolve_handle.rs
use crate::account::AccountManager;
use crate::xrpc::types::SharedIdResolver;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use poem::web::Json;
use rsky_lexicon::com::atproto::identity::ResolveHandleOutput;

async fn inner_resolve_handle(
    handle: String,
    id_resolver: &SharedIdResolver,
    account_manager: &AccountManager,
) -> Result<ResolveHandleOutput> {
    // @TODO: Implement normalizeAndEnsureValidHandle()
    let mut did: Option<String> = None;
    let user = account_manager.get_account(&handle, None).await?;

    match user {
        Some(user) => did = Some(user.did),
        None => {
            let supported_handle = crate::config::env_list("PDS_SERVICE_HANDLE_DOMAINS")
                .iter()
                .any(|host| handle.ends_with(host.as_str()) || handle == host[1..]);
            if supported_handle {
                anyhow::bail!("unable to resolve handle");
            }
        }
    }

    // the appview helper branch (PDS_BSKY_APP_VIEW_URL) is deferred with
    // pipethrough; fall through to the IdResolver for non-local handles
    if did.is_none() {
        let mut lock = id_resolver.id_resolver.write().await;
        did = lock.handle.resolve(&handle).await?;
    }

    match did {
        None => anyhow::bail!("unable to resolve handle"),
        Some(did) => Ok(ResolveHandleOutput { did }),
    }
}

/// GET /xrpc/com.atproto.identity.resolveHandle?<handle>
#[poem::handler]
pub async fn resolve_handle(
    poem::web::Query(query): poem::web::Query<ResolveHandleQuery>,
    state: poem::State<SharedState>,
) -> ApiResult<Json<ResolveHandleOutput>> {
    let ResolveHandleQuery { handle } = query;
    match inner_resolve_handle(handle, &state.id_resolver, &state.account_manager).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ResolveHandleQuery {
    pub handle: String,
}
```

```rust
// pds/src/xrpc/com/atproto/identity/refresh_identity.rs
use crate::xrpc::com::atproto::identity::resolve_identity::inner_resolve_identity;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use rsky_lexicon::com::atproto::identity::{IdentityInfo, RefreshIdentityInput};

/// POST /xrpc/com.atproto.identity.refreshIdentity
#[poem::handler]
pub async fn refresh_identity(
    body: Json<RefreshIdentityInput>,
    state: poem::State<SharedState>,
) -> ApiResult<Json<IdentityInfo>> {
    let RefreshIdentityInput { identifier } = body.0;
    let info = inner_resolve_identity(identifier, true, &state.id_resolver, &state.account_manager)
        .await?;
    Ok(Json(info))
}
```

- [ ] **Step 4: Replace the identity module placeholder with the route table**

```rust
// pds/src/xrpc/com/atproto/identity/mod.rs
pub mod get_recommended_did_credentials;
pub mod refresh_identity;
pub mod request_plc_operation_signature;
pub mod resolve_did;
pub mod resolve_handle;
pub mod resolve_identity;
pub mod sign_plc_operation;
pub mod submit_plc_operation;
pub mod update_handle;

pub fn routes() -> poem::Route {
    use poem::get;
    use poem::post;
    poem::Route::new()
        .at("/resolveDid", get(resolve_did::resolve_did))
        .at("/resolveHandle", get(resolve_handle::resolve_handle))
        .at("/resolveIdentity", get(resolve_identity::resolve_identity))
        .at("/refreshIdentity", post(refresh_identity::refresh_identity))
        .at("/getRecommendedDidCredentials", get(get_recommended_did_credentials::get_recommended_did_credentials))
        .at("/updateHandle", post(update_handle::update_handle))
        .at("/submitPlcOperation", post(submit_plc_operation::submit_plc_operation))
        .at("/signPlcOperation", post(sign_plc_operation::sign_plc_operation))
        .at("/requestPlcOperationSignature", post(request_plc_operation_signature::request_plc_operation_signature))
}
```

Tasks 20–22 create the remaining referenced files; add stub handlers returning `ApiError::RuntimeError` in this commit so the tree compiles.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pds --test identity_resolve_test`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add pds/src/xrpc/com/atproto/identity pds/tests/identity_resolve_test.rs
git commit -m "feat(identity): resolveDid/resolveHandle/resolveIdentity/refreshIdentity"
```
