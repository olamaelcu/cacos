# Task 12: repo.createRecord

**Files:**
- Create: `pds/src/xrpc/com/atproto/repo/create_record.rs`
- Test: `pds/tests/repo_create_record_test.rs`

- [ ] **Step 1: Write the failing test (full create → sequence round trip)**

```rust
// pds/tests/repo_create_record_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn create_record_then_read_locally() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    // seed the actor repo so process_writes has a root to build on
    pds::actor_store::test_helpers::seed_actor_repo(
        &state.actor_store,
        state.blobstore.clone(),
        "did:plc:alice",
    )
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
            "record": {
                "$type": "app.bsky.feed.post",
                "text": "hello world",
                "createdAt": "2026-01-01T00:00:00.000Z",
            }
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["uri"].as_str().unwrap().starts_with("at://did:plc:alice/app.bsky.feed.post/"));
    assert!(body["cid"].as_str().is_some());
}

#[tokio::test]
async fn create_record_requires_auth() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.repo.createRecord")
        .body_json(&json!({
            "repo": "did:plc:bob",
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "x", "createdAt": "2026-01-01T00:00:00.000Z" }
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::UNAUTHORIZED);
}
```

> **Test seed note:** `createRecord` → `process_writes` needs an existing repo root. `create_test_account` records a fake `repo_cid` in the account row but does not create the actor repo. Tests must seed the actor repo via the actor store (Plan 03) — the `seed_actor_repo` helper defined in Task 12 Step 2 does this (adjust to Plan 03's exact API).

Run: `cargo test -p pds --test repo_create_record_test`
Expected: FAIL — handler missing.

- [ ] **Step 2: Implement `create_record.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/create_record.rs`, with the actor-repo seeding helper in a small `pds/src/actor_store/test_helpers.rs` (also reused by later repo tests).

```rust
// pds/src/actor_store/test_helpers.rs
use crate::actor_store::ActorStore;
use crate::blobstore::BlobStore;
use crate::context::PDS_REPO_SIGNING_KEYPAIR;
use std::sync::Arc;

/// Creates the actor repo (empty commit) for `did` so write handlers have a
/// root to build on. Returns the seed commit.
pub async fn seed_actor_repo(actor_store: &ActorStore, blobstore: Arc<dyn BlobStore>, did: &str) -> anyhow::Result<()> {
    if actor_store.get_repo_root().await.is_none() {
        actor_store.create(did, &PDS_REPO_SIGNING_KEYPAIR).await?;
    }
    let txn = actor_store.transact(did.to_string(), blobstore).await?;
    txn.create_repo(Vec::new()).await?;
    Ok(())
}
```

```rust
// pds/src/xrpc/com/atproto/repo/create_record.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::AccountManager;
use crate::actor_store::ActorStore;
use crate::blobstore::BlobStore;
use crate::repo::prepare::{prepare_create, prepare_delete, PrepareCreateOpts, PrepareDeleteOpts};
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use lexicon_cid::Cid;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::repo::{CreateRecordInput, CreateRecordOutput};
use rsky_repo::types::{PreparedDelete, PreparedWrite};
use rsky_syntax::aturi::AtUri;
use std::str::FromStr;

async fn inner_create_record(
    body: CreateRecordInput,
    auth: AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<CreateRecordOutput> {
    let CreateRecordInput {
        repo,
        collection,
        record,
        rkey,
        validate,
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
        let swap_commit_cid = match swap_commit {
            Some(swap_commit) => Some(Cid::from_str(&swap_commit)?),
            None => None,
        };
        let write = prepare_create(PrepareCreateOpts {
            did: did.clone(),
            collection: collection.clone(),
            record: serde_json::from_value(record)?,
            rkey,
            validate,
            swap_cid: None,
        })
        .await?;

        let mut actor_store = state
            .actor_store
            .transact(did.clone(), state.blobstore.clone())
            .await?;
        let backlink_conflicts: Vec<AtUri> = match validate {
            Some(true) => {
                let write_at_uri: AtUri = write.uri.clone().try_into()?;
                actor_store
                    .record
                    .get_backlink_conflicts(&write_at_uri, &write.record)
                    .await?
            }
            _ => Vec::new(),
        };

        let backlink_deletions: Vec<PreparedDelete> = backlink_conflicts
            .iter()
            .map(|at_uri| {
                prepare_delete(PrepareDeleteOpts {
                    did: at_uri.get_hostname().to_string(),
                    collection: at_uri.get_collection(),
                    rkey: at_uri.get_rkey(),
                    swap_cid: None,
                })
            })
            .collect::<Result<Vec<PreparedDelete>>>()?;
        let mut writes: Vec<PreparedWrite> = vec![PreparedWrite::Create(write.clone())];
        for delete in backlink_deletions {
            writes.push(PreparedWrite::Delete(delete));
        }
        let commit = actor_store.process_writes(writes.clone(), swap_commit_cid).await?;

        let mut lock = state.sequencer.sequencer.write().await;
        lock.sequence_commit(did.clone(), commit.clone()).await?;
        state
            .account_manager
            .update_repo_root(did, commit.commit_data.cid, commit.commit_data.rev)
            .await?;

        Ok(CreateRecordOutput {
            uri: write.uri.clone(),
            cid: write.cid.to_string(),
        })
    } else {
        bail!("Could not find repo: `{repo}`")
    }
}

/// POST /xrpc/com.atproto.repo.createRecord
#[poem::handler]
pub async fn create_record(
    body: Json<CreateRecordInput>,
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<Json<CreateRecordOutput>> {
    tracing::debug!("create_record {body:?}");
    match inner_create_record(body.0, auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test repo_create_record_test`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/repo/create_record.rs pds/src/actor_store/test_helpers.rs pds/tests/repo_create_record_test.rs
git commit -m "feat(repo): createRecord handler with backlink-conflict handling"
```
