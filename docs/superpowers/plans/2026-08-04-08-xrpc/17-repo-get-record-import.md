# Task 17: repo.getRecord (local branch) + importRepo

**Files:**
- Create: `pds/src/xrpc/com/atproto/repo/get_record.rs`
- Create: `pds/src/xrpc/com/atproto/repo/import_repo.rs`
- Test: `pds/tests/repo_get_import_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/repo_get_import_test.rs
use pds::actor_store::test_helpers::seed_actor_repo;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn get_record_local_round_trip() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/com.atproto.repo.createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": "did:plc:alice",
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "round trip", "createdAt": "2026-01-01T00:00:00.000Z" }
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let created: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let uri = created["uri"].as_str().unwrap();
    let cid = created["cid"].as_str().unwrap();
    let (_, path) = uri.split_once("at://").unwrap();
    let mut parts = path.split('/');
    let repo = parts.next().unwrap();
    let collection = parts.next().unwrap();
    let rkey = parts.next().unwrap();

    let resp = cli
        .get(format!(
            "/xrpc/com.atproto.repo.getRecord?repo={repo}&collection={collection}&rkey={rkey}&cid={cid}"
        ))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["uri"], uri);
    assert_eq!(body["value"]["text"], "round trip");
}

#[tokio::test]
async fn get_record_missing_is_record_not_found() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.repo.getRecord?repo=did:plc:alice&collection=app.bsky.feed.post&rkey=3jzfcijpj2z2a")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "RecordNotFound");
}
```

Run: `cargo test -p pds --test repo_get_import_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `get_record.rs` (local branch only)**

The reference's appview pipethrough fallback is deferred (decision in header). Local repos serve from the actor_store record reader; unknown repos return `RecordNotFound`.

```rust
// pds/src/xrpc/com/atproto/repo/get_record.rs
use crate::account::AccountManager;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::repo::GetRecordOutput;
use rsky_syntax::aturi::AtUri;

async fn inner_get_record(
    repo: String,
    collection: String,
    rkey: String,
    cid: Option<String>,
    state: &SharedState,
) -> Result<GetRecordOutput> {
    let did = state.account_manager.get_did_for_actor(&repo, None).await?;

    // Local-hosted branch only; the appview pipethrough fallback is deferred.
    if let Some(did) = did {
        let uri = AtUri::make(did.clone(), Some(collection), Some(rkey))?;

        let mut actor_store = state
            .actor_store
            .read(did.clone(), state.blobstore.clone())
            .await?;

        match actor_store.record.get_record(&uri, cid, None).await {
            Ok(Some(record)) if record.takedown_ref.is_none() => Ok(GetRecordOutput {
                uri: uri.to_string(),
                cid: Some(record.cid),
                value: serde_json::to_value(record.value)?,
            }),
            _ => bail!("Could not locate record: `{uri}`"),
        }
    } else {
        bail!("Could not locate record")
    }
}

/// GET /xrpc/com.atproto.repo.getRecord?<repo>&<collection>&<rkey>&<cid>
#[poem::handler]
pub async fn get_record(
    poem::web::Query(query): poem::web::Query<GetRecordQuery>,
    state: State<SharedState>,
) -> ApiResult<Json<GetRecordOutput>> {
    let GetRecordQuery {
        repo,
        collection,
        rkey,
        cid,
    } = query;
    match inner_get_record(repo, collection, rkey, cid, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RecordNotFound)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetRecordQuery {
    pub repo: String,
    pub collection: String,
    pub rkey: String,
    pub cid: Option<String>,
}
```

- [ ] **Step 3: Implement `import_repo.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/import_repo.rs`. Rocket's `FromData` guard for the CAR body becomes a poem `Bytes` body + content-length checks.

```rust
// pds/src/xrpc/com/atproto/repo/import_repo.rs
use crate::repo::prepare::{
    prepare_create, prepare_delete, prepare_update, PrepareCreateOpts, PrepareDeleteOpts,
    PrepareUpdateOpts,
};
use crate::xrpc::auth_extractors::AccessFullImport;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use futures::{stream, StreamExt};
use lexicon_cid::Cid;
use poem::http::header::CONTENT_LENGTH;
use poem::web::Bytes;
use poem::Request;
use poem::State;
use rsky_common::env::env_int;
use rsky_repo::block_map::BlockMap;
use rsky_repo::car::{read_stream_car_with_root, CarWithRoot};
use rsky_repo::parse::get_and_parse_record;
use rsky_repo::repo::Repo;
use rsky_repo::sync::consumer::{verify_diff, VerifyRepoInput};
use rsky_repo::types::{PreparedWrite, RecordWriteDescript, VerifiedDiff};
use std::num::NonZeroU64;

async fn inner_import_repo(
    body: Bytes,
    state: &SharedState,
    requester: String,
) -> Result<(), ApiError> {
    let max_import_size = env_int("IMPORT_REPO_LIMIT").unwrap_or(100) * 1024 * 1024;
    if body.len() as u64 > max_import_size as u64 {
        return Err(ApiError::InvalidRequest(format!(
            "Content-Length is greater than maximum of {max_import_size}"
        )));
    }
    let car_with_root: CarWithRoot = read_stream_car_with_root(body.to_vec().as_slice())
        .await
        .map_err(|error| ApiError::InvalidRequest(error.to_string()))?;

    let mut actor_store = state
        .actor_store
        .transact(requester.clone(), state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    // Get current repo if it exists
    let curr_root: Option<Cid> = actor_store.get_repo_root().await;
    let curr_repo: Option<Repo> = match curr_root {
        None => None,
        Some(_root) => Some(Repo::load(actor_store.storage.clone(), curr_root).await.map_err(|_| ApiError::RuntimeError)?),
    };

    // Process imported car
    let mut imported_blocks: BlockMap = car_with_root.blocks;
    let imported_root: Cid = car_with_root.root;
    let opts = VerifyRepoInput {
        ensure_leaves: Some(false),
    };

    let diff: VerifiedDiff = verify_diff(
        curr_repo,
        &mut imported_blocks,
        imported_root,
        None,
        None,
        Some(opts),
    )
    .await
    .map_err(|error| {
        tracing::error!("{error:?}");
        ApiError::RuntimeError
    })?;

    let commit_data = diff.commit;
    let prepared_writes: Vec<PreparedWrite> =
        prepare_import_repo_writes(requester, diff.writes, &imported_blocks).await?;
    match actor_store
        .process_import_repo(commit_data, prepared_writes)
        .await
    {
        Ok(_res) => {}
        Err(error) => {
            tracing::error!("Error importing repo\n{error}");
            return Err(ApiError::RuntimeError);
        }
    }

    Ok(())
}

/// Converts list of RecordWriteDescripts into a list of PreparedWrites
async fn prepare_import_repo_writes(
    did: String,
    writes: Vec<RecordWriteDescript>,
    blocks: &BlockMap,
) -> Result<Vec<PreparedWrite>, ApiError> {
    stream::iter(writes)
        .then(|write| {
            let did = did.clone();
            async move {
                Ok::<PreparedWrite, anyhow::Error>(match write {
                    RecordWriteDescript::Create(write) => {
                        let parsed_record = get_and_parse_record(blocks, write.cid)?;
                        PreparedWrite::Create(
                            prepare_create(PrepareCreateOpts {
                                did: did.clone(),
                                collection: write.collection,
                                rkey: Some(write.rkey),
                                swap_cid: None,
                                record: parsed_record.record,
                                validate: Some(true),
                            })
                            .await?,
                        )
                    }
                    RecordWriteDescript::Update(write) => {
                        let parsed_record = get_and_parse_record(blocks, write.cid)?;
                        PreparedWrite::Update(
                            prepare_update(PrepareUpdateOpts {
                                did: did.clone(),
                                collection: write.collection,
                                rkey: write.rkey,
                                swap_cid: None,
                                record: parsed_record.record,
                                validate: Some(true),
                            })
                            .await?,
                        )
                    }
                    RecordWriteDescript::Delete(write) => {
                        PreparedWrite::Delete(prepare_delete(PrepareDeleteOpts {
                            did: did.clone(),
                            collection: write.collection,
                            rkey: write.rkey,
                            swap_cid: None,
                        })?)
                    }
                })
            }
        })
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<anyhow::Result<Vec<PreparedWrite>, _>>()
        .map_err(|error| {
            tracing::error!("Error preparing import repo writes\n{error}");
            ApiError::RuntimeError
        })
}

/// POST /xrpc/com.atproto.repo.importRepo
#[poem::handler]
pub async fn import_repo(
    auth: AccessFullImport,
    body: Bytes,
    req: &Request,
    state: State<SharedState>,
) -> ApiResult<()> {
    let requester = auth.access.credentials.unwrap().did.unwrap();
    // content-length guard (mirrors the reference FromData impl)
    if req.headers().get(CONTENT_LENGTH).is_none() {
        return Err(ApiError::InvalidRequest(
            "Missing content-length header".to_string(),
        ));
    }
    match inner_import_repo(body, &state, requester).await {
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pds --test repo_get_import_test`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add pds/src/xrpc/com/atproto/repo/get_record.rs pds/src/xrpc/com/atproto/repo/import_repo.rs pds/tests/repo_get_import_test.rs
git commit -m "feat(repo): getRecord (local branch) and importRepo"
```
