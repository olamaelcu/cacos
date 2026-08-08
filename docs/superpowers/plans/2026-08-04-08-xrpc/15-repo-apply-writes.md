# Task 15: repo.applyWrites

**Files:**
- Create: `pds/src/xrpc/com/atproto/repo/apply_writes.rs`
- Test: `pds/tests/repo_apply_writes_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
// pds/tests/repo_apply_writes_test.rs
use pds::actor_store::test_helpers::seed_actor_repo;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn apply_writes_creates_two_records() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/com.atproto.repo.applyWrites")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": "did:plc:alice",
            "writes": [
                {
                    "$type": "com.atproto.repo.applyWrites#create",
                    "collection": "app.bsky.feed.post",
                    "value": { "$type": "app.bsky.feed.post", "text": "one", "createdAt": "2026-01-01T00:00:00.000Z" }
                },
                {
                    "$type": "com.atproto.repo.applyWrites#create",
                    "collection": "app.bsky.feed.post",
                    "value": { "$type": "app.bsky.feed.post", "text": "two", "createdAt": "2026-01-01T00:00:00.000Z" }
                }
            ]
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    let resp = cli
        .get("/xrpc/com.atproto.repo.listRecords?repo=did:plc:alice&collection=app.bsky.feed.post")
        .send()
        .await;
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["records"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn apply_writes_too_many_rejected() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);

    let writes: Vec<serde_json::Value> = (0..201)
        .map(|i| {
            json!({
                "$type": "com.atproto.repo.applyWrites#create",
                "collection": "app.bsky.feed.post",
                "value": { "$type": "app.bsky.feed.post", "text": format!("p{i}"), "createdAt": "2026-01-01T00:00:00.000Z" }
            })
        })
        .collect();
    let resp = cli
        .post("/xrpc/com.atproto.repo.applyWrites")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "repo": "did:plc:alice", "writes": writes }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
}
```

Run: `cargo test -p pds --test repo_apply_writes_test`
Expected: FAIL — handler missing.

- [ ] **Step 2: Implement `apply_writes.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/apply_writes.rs`.

```rust
// pds/src/xrpc/com/atproto/repo/apply_writes.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::AccountManager;
use crate::repo::prepare::{
    prepare_create, prepare_delete, prepare_update, PrepareCreateOpts, PrepareDeleteOpts,
    PrepareUpdateOpts,
};
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use futures::stream::{self, StreamExt};
use lexicon_cid::Cid;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::repo::{ApplyWritesInput, ApplyWritesInputRefWrite};
use rsky_repo::types::PreparedWrite;
use std::str::FromStr;

async fn inner_apply_writes(
    body: ApplyWritesInput,
    auth: AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<()> {
    let ApplyWritesInput {
        repo,
        validate,
        swap_commit,
        writes: tx_writes,
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
        if tx_writes.len() > 200 {
            bail!("Too many writes. Max: 200")
        }

        let writes: Vec<PreparedWrite> = stream::iter(tx_writes)
            .then(|write| async move {
                Ok::<PreparedWrite, anyhow::Error>(match write {
                    ApplyWritesInputRefWrite::Create(write) => PreparedWrite::Create(
                        prepare_create(PrepareCreateOpts {
                            did: did.clone(),
                            collection: write.collection,
                            rkey: write.rkey,
                            swap_cid: None,
                            record: serde_json::from_value(write.value)?,
                            validate,
                        })
                        .await?,
                    ),
                    ApplyWritesInputRefWrite::Update(write) => PreparedWrite::Update(
                        prepare_update(PrepareUpdateOpts {
                            did: did.clone(),
                            collection: write.collection,
                            rkey: write.rkey,
                            swap_cid: None,
                            record: serde_json::from_value(write.value)?,
                            validate,
                        })
                        .await?,
                    ),
                    ApplyWritesInputRefWrite::Delete(write) => {
                        PreparedWrite::Delete(prepare_delete(PrepareDeleteOpts {
                            did: did.clone(),
                            collection: write.collection,
                            rkey: write.rkey,
                            swap_cid: None,
                        })?)
                    }
                })
            })
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<PreparedWrite>, _>>()?;

        let swap_commit_cid = match swap_commit {
            Some(swap_commit) => Some(Cid::from_str(&swap_commit)?),
            None => None,
        };

        let mut actor_store = state
            .actor_store
            .transact(did.clone(), state.blobstore.clone())
            .await?;

        let commit = actor_store.process_writes(writes.clone(), swap_commit_cid).await?;

        let mut lock = state.sequencer.sequencer.write().await;
        lock.sequence_commit(did.clone(), commit.clone()).await?;
        state
            .account_manager
            .update_repo_root(did, commit.commit_data.cid, commit.commit_data.rev)
            .await?;
        Ok(())
    } else {
        bail!("Could not find repo: `{repo}`")
    }
}

/// POST /xrpc/com.atproto.repo.applyWrites
#[poem::handler]
pub async fn apply_writes(
    body: Json<ApplyWritesInput>,
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_apply_writes(body.0, auth, &state).await {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

> **Lexicon note:** if `ApplyWritesInput` destructures with `..` for the `writes` field name in the version of rsky-lexicon pinned, keep the reference's field-name handling (`repo, validate, swap_commit, writes`).

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test repo_apply_writes_test`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/repo/apply_writes.rs pds/tests/repo_apply_writes_test.rs
git commit -m "feat(repo): applyWrites handler"
```
