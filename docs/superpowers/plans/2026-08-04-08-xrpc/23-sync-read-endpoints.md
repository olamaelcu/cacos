# Task 23: Sync read endpoints — getRepo, getCheckout, getBlocks, getRecord(sync), getHead, getLatestCommit, getBlob

**Files:**
- Create: `pds/src/xrpc/com/atproto/sync/mod.rs` (route table; replace Task 1 placeholder)
- Create: `pds/src/xrpc/com/atproto/sync/get_repo.rs`
- Create: `pds/src/xrpc/com/atproto/sync/get_checkout.rs`
- Create: `pds/src/xrpc/com/atproto/sync/get_blocks.rs`
- Create: `pds/src/xrpc/com/atproto/sync/get_record.rs`
- Create: `pds/src/xrpc/com/atproto/sync/get_head.rs`
- Create: `pds/src/xrpc/com/atproto/sync/get_latest_commit.rs`
- Create: `pds/src/xrpc/com/atproto/sync/get_blob.rs`
- Test: `pds/tests/sync_read_test.rs`

- [ ] **Step 1: Write the failing test (getRepo round trip)**

```rust
// pds/tests/sync_read_test.rs
use pds::actor_store::test_helpers::seed_actor_repo;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn get_repo_returns_car() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    // put one record so the CAR has content
    let app = build_app(state);
    let cli = TestClient::new(app);
    let _ = cli
        .post("/xrpc/com.atproto.repo.createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": "did:plc:alice",
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "sync me", "createdAt": "2026-01-01T00:00:00.000Z" }
        }))
        .send()
        .await;

    let resp = cli
        .get("/xrpc/com.atproto.sync.getRepo?did=did:plc:alice")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert_eq!(resp.0.content_type().unwrap_or_default(), "application/vnd.ipld.car");
    let bytes = resp.0.into_body().into_bytes().await.unwrap();
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn get_latest_commit_returns_cid_and_rev() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.sync.getLatestCommit?did=did:plc:alice")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["cid"].as_str().is_some());
    assert!(body["rev"].as_str().is_some());
}

#[tokio::test]
async fn get_head_returns_root() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.sync.getHead?did=did:plc:alice")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["root"].as_str().is_some());
}
```

> **Test note:** these three tests exercise the shared `assert_repo_availability` + `is_user_or_admin` paths. `get_blocks`/`get_record`(sync)/`get_checkout`/`get_blob` share the same shape; their assertions follow the same pattern (content-type `application/vnd.ipld.car`, 404 shape on missing repo → RuntimeError per reference).

Run: `cargo test -p pds --test sync_read_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement the sync module + CAR-returning helpers**

A shared helper builds poem `Response` bodies for CAR bytes (used by all CAR endpoints).

```rust
// pds/src/xrpc/com/atproto/sync/car_responder.rs
use poem::http::StatusCode;
use poem::Response;

pub fn car_response(bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .content_type("application/vnd.ipld.car")
        .body(bytes)
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_repo.rs
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::com::atproto::sync::car_responder::car_response;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::State;

async fn get_car_stream(
    state: &SharedState,
    did: String,
    since: Option<String>,
) -> Result<Vec<u8>> {
    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await?;
    let storage_guard = actor_store.storage.read().await;
    match storage_guard.get_car_stream(since).await {
        Err(_) => bail!("Could not find repo for DID: {did}"),
        Ok(carstream) => Ok(carstream),
    }
}

async fn inner_get_repo(
    did: String,
    since: Option<String>,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<Vec<u8>> {
    let is_user_or_admin = if let Some(access) = auth.access {
        crate::auth::auth_verifier::is_user_or_admin(&access, &did)
    } else {
        false
    };
    let _ = assert_repo_availability(&did, is_user_or_admin, &state.account_manager).await?;
    get_car_stream(state, did, since).await
}

/// GET /xrpc/com.atproto.sync.getRepo?<did>&<since>
#[poem::handler]
pub async fn get_repo(
    poem::web::Query(query): poem::web::Query<GetRepoQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<poem::Response> {
    let GetRepoQuery { did, since } = query;
    match inner_get_repo(did, since, auth, &state).await {
        Ok(res) => Ok(car_response(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetRepoQuery {
    pub did: String,
    pub since: Option<String>,
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_checkout.rs
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::com::atproto::sync::car_responder::car_response;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::State;

async fn inner_get_checkout(
    did: String,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<Vec<u8>> {
    let is_user_or_admin = if let Some(access) = auth.access {
        crate::auth::auth_verifier::is_user_or_admin(&access, &did)
    } else {
        false
    };
    let _ = assert_repo_availability(&did, is_user_or_admin, &state.account_manager).await?;
    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await?;
    let storage_guard = actor_store.storage.read().await;
    match storage_guard.get_car_stream(None).await {
        Err(_) => bail!("Could not find repo for DID: {did}"),
        Ok(carstream) => Ok(carstream),
    }
}

/// DEPRECATED — GET /xrpc/com.atproto.sync.getCheckout?<did>
#[poem::handler]
pub async fn get_checkout(
    poem::web::Query(query): poem::web::Query<GetCheckoutQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<poem::Response> {
    let GetCheckoutQuery { did } = query;
    match inner_get_checkout(did, auth, &state).await {
        Ok(res) => Ok(car_response(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetCheckoutQuery {
    pub did: String,
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_blocks.rs
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::com::atproto::sync::car_responder::car_response;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use lexicon_cid::Cid;
use poem::State;
use rsky_repo::car::blocks_to_car_file;
use rsky_repo::storage::readable_blockstore::ReadableBlockstore;
use std::str::FromStr;

async fn inner_get_blocks(
    did: String,
    cids: Vec<String>,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<Vec<u8>> {
    let is_user_or_admin = if let Some(access) = auth.access {
        crate::auth::auth_verifier::is_user_or_admin(&access, &did)
    } else {
        false
    };
    let _ = assert_repo_availability(&did, is_user_or_admin, &state.account_manager).await?;

    let cids: Vec<Cid> = cids
        .into_iter()
        .map(|c| Cid::from_str(&c).map_err(anyhow::Error::new))
        .collect::<Result<Vec<Cid>>>()?;

    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await?;
    let storage_guard = actor_store.storage.read().await;
    let got = storage_guard.get_blocks(cids).await?;

    if !got.missing.is_empty() {
        let missing_str = got
            .missing
            .into_iter()
            .map(|c| c.to_string())
            .collect::<Vec<String>>();
        bail!("Could not find cids: `{missing_str:?}`");
    }

    let car = blocks_to_car_file(None, got.blocks).await?;
    Ok(car)
}

/// GET /xrpc/com.atproto.sync.getBlocks?<did>&<cids>
#[poem::handler]
pub async fn get_blocks(
    poem::web::Query(query): poem::web::Query<GetBlocksQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<poem::Response> {
    let GetBlocksQuery { did, cids } = query;
    match inner_get_blocks(did, cids, auth, &state).await {
        Ok(res) => Ok(car_response(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetBlocksQuery {
    pub did: String,
    pub cids: Vec<String>,
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_record.rs
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::com::atproto::sync::car_responder::car_response;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use lexicon_cid::Cid;
use poem::State;
use rsky_repo::types::RecordPath;
use std::str::FromStr;

async fn inner_get_record(
    did: String,
    collection: String,
    rkey: String,
    commit: Option<String>,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<Vec<u8>> {
    let is_user_or_admin = if let Some(access) = auth.access {
        crate::auth::auth_verifier::is_user_or_admin(&access, &did)
    } else {
        false
    };
    let _ = assert_repo_availability(&did, is_user_or_admin, &state.account_manager).await?;
    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await?;
    let storage_guard = actor_store.storage.read().await;
    let commit: Option<Cid> = match commit {
        Some(commit) => Some(Cid::from_str(&commit)?),
        None => storage_guard.get_root().await,
    };

    match commit {
        None => bail!("Could not find repo for DID: {did}"),
        Some(commit) => {
            rsky_repo::sync::provider::get_records(
                actor_store.storage.clone(),
                commit,
                vec![RecordPath { collection, rkey }],
            )
            .await
        }
    }
}

/// GET /xrpc/com.atproto.sync.getRecord?<did>&<collection>&<rkey>&<commit>
#[poem::handler]
pub async fn get_record(
    poem::web::Query(query): poem::web::Query<GetRecordQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<poem::Response> {
    let GetRecordQuery {
        did,
        collection,
        rkey,
        commit,
    } = query;
    match inner_get_record(did, collection, rkey, commit, auth, &state).await {
        Ok(res) => Ok(car_response(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetRecordQuery {
    pub did: String,
    pub collection: String,
    pub rkey: String,
    pub commit: Option<String>,
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_head.rs
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::sync::GetHeadOutput;

async fn inner_get_head(
    did: String,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<GetHeadOutput, ApiError> {
    let is_user_or_admin = if let Some(access) = auth.access {
        crate::auth::auth_verifier::is_user_or_admin(&access, &did)
    } else {
        false
    };
    let _ = assert_repo_availability(&did, is_user_or_admin, &state.account_manager)
        .await
        .map_err(|error| {
            tracing::error!("{error}");
            ApiError::RuntimeError
        })?;
    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await
        .map_err(|error| {
            tracing::error!("{error}");
            ApiError::RuntimeError
        })?;
    let storage_guard = actor_store.storage.read().await;
    match storage_guard.get_root_detailed().await {
        Ok(root) => Ok(GetHeadOutput {
            root: root.cid.to_string(),
        }),
        Err(_) => Err(ApiError::BadRequest(
            "HeadNotFound".to_string(),
            format!("Could not find root for DID: {did}"),
        )),
    }
}

/// DEPRECATED — GET /xrpc/com.atproto.sync.getHead?<did>
#[poem::handler]
pub async fn get_head(
    poem::web::Query(query): poem::web::Query<GetHeadQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<Json<GetHeadOutput>> {
    let GetHeadQuery { did } = query;
    let res = inner_get_head(did, auth, &state).await?;
    Ok(Json(res))
}

#[derive(serde::Deserialize)]
pub struct GetHeadQuery {
    pub did: String,
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_latest_commit.rs
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::sync::GetLatestCommitOutput;

async fn inner_get_latest_commit(
    did: String,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<GetLatestCommitOutput> {
    let is_user_or_admin = if let Some(access) = auth.access {
        crate::auth::auth_verifier::is_user_or_admin(&access, &did)
    } else {
        false
    };
    let _ = assert_repo_availability(&did, is_user_or_admin, &state.account_manager).await?;

    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await?;
    let storage_guard = actor_store.storage.read().await;
    match storage_guard.get_root_detailed().await {
        Ok(res) => Ok(GetLatestCommitOutput {
            cid: res.cid.to_string(),
            rev: res.rev,
        }),
        Err(_) => bail!("Could not find root for DID: {did}"),
    }
}

/// GET /xrpc/com.atproto.sync.getLatestCommit?<did>
#[poem::handler]
pub async fn get_latest_commit(
    poem::web::Query(query): poem::web::Query<GetLatestCommitQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<Json<GetLatestCommitOutput>> {
    let GetLatestCommitQuery { did } = query;
    match inner_get_latest_commit(did, auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetLatestCommitQuery {
    pub did: String,
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_blob.rs
use crate::account::AccountManager;
use crate::blobstore::BlobNotFoundError;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use lexicon_cid::Cid;
use poem::http::header::CONTENT_TYPE;
use poem::http::StatusCode;
use poem::Response;
use poem::State;
use std::str::FromStr;

async fn inner_get_blob(
    did: String,
    cid: String,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<(Vec<u8>, Option<String>)> {
    let is_user_or_admin = if let Some(access) = auth.access {
        crate::auth::auth_verifier::is_user_or_admin(&access, &did)
    } else {
        false
    };
    let _ = assert_repo_availability(&did, is_user_or_admin, &state.account_manager).await?;

    let cid = Cid::from_str(&cid)?;
    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await?;

    let found = actor_store.blob.get_blob(cid).await?;
    let bytes: Vec<u8> = found.stream.to_vec();
    Ok((bytes, found.mime_type))
}

/// GET /xrpc/com.atproto.sync.getBlob?<did>&<cid>
#[poem::handler]
pub async fn get_blob(
    poem::web::Query(query): poem::web::Query<GetBlobQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<Response> {
    let GetBlobQuery { did, cid } = query;
    match inner_get_blob(did, cid, auth, &state).await {
        Ok((bytes, mime_type)) => {
            let builder = Response::builder()
                .status(StatusCode::OK)
                .header("content-length", bytes.len().to_string())
                .header(
                    CONTENT_TYPE,
                    mime_type.unwrap_or("application/octet-stream".to_string()),
                )
                .header("content-security-policy", "default-src 'none'; sandbox");
            Ok(builder.body(bytes))
        }
        Err(error) => {
            tracing::error!("Error: {error}");
            if error.downcast_ref::<BlobNotFoundError>().is_some() {
                Err(ApiError::BlobNotFound)
            } else {
                Err(ApiError::RuntimeError)
            }
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetBlobQuery {
    pub did: String,
    pub cid: String,
}
```

> **Plan 04 note:** `BlobNotFoundError` and the blob stream type (`found.stream: Vec<u8>` after collect, per the forked `BlobStore` returning `Bytes`) come from Plan 04's forked trait. If `BlobReader::get_blob` returns a different stream type, adjust `inner_get_blob`'s collect call.

- [ ] **Step 3: Replace the sync module placeholder with the route table**

```rust
// pds/src/xrpc/com/atproto/sync/mod.rs
pub mod car_responder;
pub mod get_blob;
pub mod get_blocks;
pub mod get_checkout;
pub mod get_head;
pub mod get_latest_commit;
pub mod get_record;
pub mod get_repo;
pub mod get_repo_status;
pub mod list_blobs;
pub mod list_repos;
// subscribe_repos.rs is Plan 07's file in this same module tree — declared
// here (if Plan 07 did not already declare it in sync/mod.rs) and mounted
// below. Its handler extracts State<SharedSequencer> and
// State<SharedBroadcast>; build_app registers those types (Task 1).
pub mod subscribe_repos;

pub fn routes() -> poem::Route {
    use poem::get;
    poem::Route::new()
        .at("/getRepo", get(get_repo::get_repo))
        .at("/getCheckout", get(get_checkout::get_checkout))
        .at("/getBlocks", get(get_blocks::get_blocks))
        .at("/getRecord", get(get_record::get_record))
        .at("/getHead", get(get_head::get_head))
        .at("/getLatestCommit", get(get_latest_commit::get_latest_commit))
        .at("/getBlob", get(get_blob::get_blob))
        .at("/listBlobs", get(list_blobs::list_blobs))
        .at("/listRepos", get(list_repos::list_repos))
        .at("/getRepoStatus", get(get_repo_status::get_repo_status))
        .at("/subscribeRepos", get(subscribe_repos::subscribe_repos))
}
```

Tasks 24 creates `list_blobs`, `list_repos`, `get_repo_status`; add stub handlers for this commit so the tree compiles.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pds --test sync_read_test`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add pds/src/xrpc/com/atproto/sync pds/tests/sync_read_test.rs
git commit -m "feat(sync): CAR read endpoints (getRepo/Checkout/Blocks/Record/Head/LatestCommit/Blob)"
```
