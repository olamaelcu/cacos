# Task 13: repo.putRecord

**Files:**
- Create: `pds/src/xrpc/com/atproto/repo/put_record.rs`
- Test: `pds/tests/repo_put_record_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
// pds/tests/repo_put_record_test.rs
use pds::actor_store::test_helpers::seed_actor_repo;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn put_record_creates_then_updates() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);

    let rkey = "3jzfcijpj2z2a";
    let resp = cli
        .post("/xrpc/com.atproto.repo.putRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": "did:plc:alice",
            "collection": "app.bsky.feed.post",
            "rkey": rkey,
            "record": {
                "$type": "app.bsky.feed.post",
                "text": "first",
                "createdAt": "2026-01-01T00:00:00.000Z",
            }
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["uri"].as_str().unwrap().ends_with(rkey));
    let first_cid = body["cid"].as_str().unwrap().to_string();

    // updating the same rkey returns a new cid (update path)
    let resp = cli
        .post("/xrpc/com.atproto.repo.putRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": "did:plc:alice",
            "collection": "app.bsky.feed.post",
            "rkey": rkey,
            "record": {
                "$type": "app.bsky.feed.post",
                "text": "second",
                "createdAt": "2026-01-01T00:00:00.000Z",
            }
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_ne!(body["cid"].as_str().unwrap(), first_cid);
}
```

Run: `cargo test -p pds --test repo_put_record_test`
Expected: FAIL — handler missing.

- [ ] **Step 2: Implement `put_record.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/put_record.rs`.

```rust
// pds/src/xrpc/com/atproto/repo/put_record.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::AccountManager;
use crate::actor_store::ActorStore;
use crate::repo::prepare::{prepare_create, prepare_update, PrepareCreateOpts, PrepareUpdateOpts};
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use lexicon_cid::Cid;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::repo::{PutRecordInput, PutRecordOutput};
use rsky_repo::types::{CommitDataWithOps, PreparedWrite};
use rsky_syntax::aturi::AtUri;
use std::str::FromStr;

async fn inner_put_record(
    body: PutRecordInput,
    auth: AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<PutRecordOutput> {
    let PutRecordInput {
        repo,
        collection,
        rkey,
        validate,
        record,
        swap_record,
        swap_commit,
    } = body;
    let account = state
        .account_manager
        .get_account(
            &repo,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await?;
    if let Some(account) = account {
        if account.deactivated_at.is_some() {
            bail!("Account is deactivated")
        }
        let did = account.did;
        if did != auth.access.credentials.unwrap().did.unwrap() {
            bail!("AuthRequiredError")
        }
        let uri = AtUri::make(did.clone(), Some(collection.clone()), Some(rkey.clone()))?;
        let swap_commit_cid = match swap_commit {
            Some(swap_commit) => Some(Cid::from_str(&swap_commit)?),
            None => None,
        };
        let swap_record_cid = match swap_record {
            Some(swap_record) => Some(Cid::from_str(&swap_record)?),
            None => None,
        };
        let (commit, write): (Option<CommitDataWithOps>, PreparedWrite) = {
            let mut actor_store = state
                .actor_store
                .transact(did.clone(), state.blobstore.clone())
                .await?;

            let current = actor_store
                .record
                .get_record(&uri, None, Some(true))
                .await?;
            let write: PreparedWrite = if current.is_some() {
                PreparedWrite::Update(
                    prepare_update(PrepareUpdateOpts {
                        did: did.clone(),
                        collection,
                        rkey,
                        swap_cid: swap_record_cid,
                        record: serde_json::from_value(record)?,
                        validate,
                    })
                    .await?,
                )
            } else {
                PreparedWrite::Create(
                    prepare_create(PrepareCreateOpts {
                        did: did.clone(),
                        collection,
                        rkey: Some(rkey),
                        swap_cid: swap_record_cid,
                        record: serde_json::from_value(record)?,
                        validate,
                    })
                    .await?,
                )
            };

            match current {
                Some(current) if current.cid == write.cid().unwrap().to_string() => (None, write),
                _ => {
                    let commit = actor_store
                        .process_writes(vec![write.clone()], swap_commit_cid)
                        .await?;
                    (Some(commit), write)
                }
            }
        };

        if let Some(commit) = commit {
            let mut lock = state.sequencer.sequencer.write().await;
            lock.sequence_commit(did.clone(), commit.clone()).await?;
            state
                .account_manager
                .update_repo_root(did, commit.commit_data.cid, commit.commit_data.rev)
                .await?;
        }
        Ok(PutRecordOutput {
            uri: write.uri().to_string(),
            cid: write.cid().unwrap().to_string(),
        })
    } else {
        bail!("Could not find repo: `{repo}`")
    }
}

/// POST /xrpc/com.atproto.repo.putRecord
#[poem::handler]
pub async fn put_record(
    body: Json<PutRecordInput>,
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<Json<PutRecordOutput>> {
    match inner_put_record(body.0, auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test repo_put_record_test`
Expected: PASS (1 test).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/repo/put_record.rs pds/tests/repo_put_record_test.rs
git commit -m "feat(repo): putRecord handler"
```
