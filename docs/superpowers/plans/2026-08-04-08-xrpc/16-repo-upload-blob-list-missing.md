# Task 16: repo.uploadBlob + listMissingBlobs

**Files:**
- Create: `pds/src/xrpc/com/atproto/repo/upload_blob.rs`
- Create: `pds/src/xrpc/com/atproto/repo/list_missing_blobs.rs`
- Test: `pds/tests/repo_blob_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/repo_blob_test.rs
use pds::actor_store::test_helpers::seed_actor_repo;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;

#[tokio::test]
async fn upload_blob_then_list_missing() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);

    let bytes = b"hello blob bytes".to_vec();
    let resp = cli
        .post("/xrpc/com.atproto.repo.uploadBlob")
        .header("Authorization", format!("Bearer {access}"))
        .content_type("image/png")
        .body_bytes(bytes)
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["blob"]["mimeType"], "image/png");
    assert!(body["blob"]["ref"]["$link"].as_str().is_some());

    let resp = cli
        .get("/xrpc/com.atproto.repo.listMissingBlobs")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    // untethered upload shows up as a missing blob (not referenced by a record)
    assert_eq!(body["blobs"].as_array().unwrap().len(), 1);
}
```

Run: `cargo test -p pds --test repo_blob_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `upload_blob.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/upload_blob.rs`. Rocket's `Data`/`ContentType` guards become poem's `Bytes` body + a `Content-Type` header read.

```rust
// pds/src/xrpc/com/atproto/repo/upload_blob.rs
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::http::header::CONTENT_TYPE;
use poem::web::Bytes;
use poem::Request;
use poem::State;
use rsky_lexicon::com::atproto::repo::{Blob, BlobOutput};
use rsky_repo::types::{BlobConstraint, PreparedBlobRef};

async fn inner_upload_blob(
    bytes: Bytes,
    content_type: String,
    state: &SharedState,
    requester: String,
) -> Result<BlobOutput, ApiError> {
    let actor_store = state
        .actor_store
        .transact(requester.clone(), state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    let metadata = actor_store
        .blob
        .upload_blob_and_get_metadata(content_type, bytes.to_vec())
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let blobref = actor_store
        .blob
        .track_untethered_blob(metadata)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    // make the blob permanent if an associated record is already indexed
    let records_for_blob = actor_store
        .blob
        .get_records_for_blob(blobref.get_cid()?)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if !records_for_blob.is_empty() {
        actor_store
            .blob
            .verify_blob_and_make_permanent(PreparedBlobRef {
                cid: blobref.get_cid()?,
                mime_type: blobref.get_mime_type().to_string(),
                constraints: BlobConstraint {
                    max_size: None,
                    accept: None,
                },
            })
            .await
            .map_err(|_| ApiError::RuntimeError)?;
    }

    Ok(BlobOutput {
        blob: Blob {
            r#type: Some("blob".to_string()),
            r#ref: Some(blobref.get_cid()?),
            cid: None,
            mime_type: blobref.get_mime_type().to_string(),
            size: blobref.get_size(),
            original: None,
        },
    })
}

/// POST /xrpc/com.atproto.repo.uploadBlob
#[poem::handler]
pub async fn upload_blob(
    auth: AccessStandardIncludeChecks,
    body: Bytes,
    req: &Request,
    state: State<SharedState>,
) -> ApiResult<poem::Response> {
    let requester = auth.access.credentials.unwrap().did.unwrap();
    let content_type = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    match inner_upload_blob(body, content_type, &state, requester).await {
        Ok(res) => Ok(poem::Response::builder()
            .status(poem::http::StatusCode::OK)
            .content_type("application/json")
            .body(serde_json::to_vec(&res).unwrap())),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Implement `list_missing_blobs.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/list_missing_blobs.rs`.

```rust
// pds/src/xrpc/com/atproto/repo/list_missing_blobs.rs
use crate::actor_store::blob::ListMissingBlobsOpts;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::repo::ListMissingBlobsOutput;

/// GET /xrpc/com.atproto.repo.listMissingBlobs?<limit>&<cursor>
#[poem::handler]
pub async fn list_missing_blobs(
    poem::web::Query(query): poem::web::Query<ListMissingBlobsQuery>,
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<Json<ListMissingBlobsOutput>> {
    let ListMissingBlobsQuery { limit, cursor } = query;
    let did = auth.access.credentials.unwrap().did.unwrap();
    let limit: u16 = limit.unwrap_or(500);

    let actor_store = state
        .actor_store
        .read(did.clone(), state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    match actor_store
        .blob
        .list_missing_blobs(ListMissingBlobsOpts { cursor, limit })
        .await
    {
        Ok(blobs) => {
            let cursor = blobs.last().map(|last_blob| last_blob.cid.clone());
            Ok(Json(ListMissingBlobsOutput { cursor, blobs }))
        }
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ListMissingBlobsQuery {
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pds --test repo_blob_test`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add pds/src/xrpc/com/atproto/repo/upload_blob.rs pds/src/xrpc/com/atproto/repo/list_missing_blobs.rs pds/tests/repo_blob_test.rs
git commit -m "feat(repo): uploadBlob and listMissingBlobs"
```
