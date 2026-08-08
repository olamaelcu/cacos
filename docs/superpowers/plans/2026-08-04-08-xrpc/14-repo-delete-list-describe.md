# Task 14: repo.deleteRecord + listRecords + describeRepo

**Files:**
- Create: `pds/src/xrpc/com/atproto/repo/delete_record.rs`
- Create: `pds/src/xrpc/com/atproto/repo/list_records.rs`
- Create: `pds/src/xrpc/com/atproto/repo/describe_repo.rs`
- Test: `pds/tests/repo_read_write_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/repo_read_write_test.rs
use pds::actor_store::test_helpers::seed_actor_repo;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

async fn create_a_post(cli: &TestClient<impl poem::Endpoint>, access: &str) -> (String, String) {
    let resp = cli
        .post("/xrpc/com.atproto.repo.createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": "did:plc:alice",
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "hello", "createdAt": "2026-01-01T00:00:00.000Z" }
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    (body["uri"].as_str().unwrap().to_string(), body["cid"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn list_records_returns_created_record() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let (uri, _cid) = create_a_post(&cli, &access).await;

    let resp = cli
        .get("/xrpc/com.atproto.repo.listRecords?repo=did:plc:alice&collection=app.bsky.feed.post&limit=50")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let records = body["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["uri"], uri);
    assert_eq!(records[0]["value"]["text"], "hello");
}

#[tokio::test]
async fn delete_record_removes_it() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    seed_actor_repo(&state.actor_store, state.blobstore.clone(), "did:plc:alice")
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let (uri, _cid) = create_a_post(&cli, &access).await;
    let (_, rkey) = uri.rsplit_once('/').unwrap();

    let resp = cli
        .post("/xrpc/com.atproto.repo.deleteRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": "did:plc:alice",
            "collection": "app.bsky.feed.post",
            "rkey": rkey,
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    let resp = cli
        .get("/xrpc/com.atproto.repo.listRecords?repo=did:plc:alice&collection=app.bsky.feed.post")
        .send()
        .await;
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["records"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn describe_repo_returns_account_info() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.repo.describeRepo?repo=did:plc:alice")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:alice");
    assert_eq!(body["handle"], "alice.test");
}
```

Run: `cargo test -p pds --test repo_read_write_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `delete_record.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/delete_record.rs`.

```rust
// pds/src/xrpc/com/atproto/repo/delete_record.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::AccountManager;
use crate::repo::prepare::{prepare_delete, PrepareDeleteOpts};
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use lexicon_cid::Cid;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::repo::DeleteRecordInput;
use rsky_repo::types::PreparedWrite;
use rsky_syntax::aturi::AtUri;
use std::str::FromStr;

async fn inner_delete_record(
    body: DeleteRecordInput,
    auth: AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<()> {
    let DeleteRecordInput {
        repo,
        collection,
        rkey,
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
    match account {
        None => bail!("Could not find repo: `{repo}`"),
        Some(account) if account.deactivated_at.is_some() => bail!("Account is deactivated"),
        Some(account) => {
            let did = account.did;
            if did != auth.access.credentials.unwrap().did.unwrap() {
                bail!("AuthRequiredError")
            }

            let swap_commit_cid = match swap_commit {
                Some(swap_commit) => Some(Cid::from_str(&swap_commit)?),
                None => None,
            };
            let swap_record_cid = match swap_record {
                Some(swap_record) => Some(Cid::from_str(&swap_record)?),
                None => None,
            };

            let write = prepare_delete(PrepareDeleteOpts {
                did: did.clone(),
                collection,
                rkey,
                swap_cid: swap_record_cid,
            })?;
            let mut actor_store = state
                .actor_store
                .transact(did.clone(), state.blobstore.clone())
                .await?;
            let write_at_uri: AtUri = write.uri.clone().try_into()?;
            let record = actor_store
                .record
                .get_record(&write_at_uri, None, Some(true))
                .await?;
            let commit = match record {
                None => return Ok(()), // No-op if record already doesn't exist
                Some(_) => {
                    actor_store
                        .process_writes(vec![PreparedWrite::Delete(write.clone())], swap_commit_cid)
                        .await?
                }
            };

            let mut lock = state.sequencer.sequencer.write().await;
            lock.sequence_commit(did.clone(), commit.clone()).await?;
            state
                .account_manager
                .update_repo_root(did, commit.commit_data.cid, commit.commit_data.rev)
                .await?;

            Ok(())
        }
    }
}

/// POST /xrpc/com.atproto.repo.deleteRecord
#[poem::handler]
pub async fn delete_record(
    body: Json<DeleteRecordInput>,
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_delete_record(body.0, auth, &state).await {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Implement `list_records.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/list_records.rs`.

```rust
// pds/src/xrpc/com/atproto/repo/list_records.rs
use crate::account::AccountManager;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::repo::{ListRecordsOutput, Record};
use rsky_syntax::aturi::AtUri;

#[allow(non_snake_case)]
async fn inner_list_records(
    repo: String,
    collection: String,
    limit: u16,
    cursor: Option<String>,
    rkeyStart: Option<String>,
    rkeyEnd: Option<String>,
    reverse: bool,
    state: &SharedState,
) -> Result<ListRecordsOutput> {
    if limit > 100 {
        bail!("Error: limit can not be greater than 100")
    }
    let did = state.account_manager.get_did_for_actor(&repo, None).await?;
    if let Some(did) = did {
        let mut actor_store = state
            .actor_store
            .read(did.clone(), state.blobstore.clone())
            .await?;

        let records: Vec<Record> = actor_store
            .record
            .list_records_for_collection(
                collection,
                limit as i64,
                reverse,
                cursor,
                rkeyStart,
                rkeyEnd,
                None,
            )
            .await?
            .into_iter()
            .map(|record| {
                Ok(Record {
                    uri: record.uri.clone(),
                    cid: record.cid.clone(),
                    value: serde_json::to_value(record)?,
                })
            })
            .collect::<Result<Vec<Record>>>()?;

        let last_record = records.last();
        let cursor: Option<String> = if let Some(last_record) = last_record {
            let last_at_uri: AtUri = last_record.uri.clone().try_into()?;
            Some(last_at_uri.get_rkey())
        } else {
            None
        };
        Ok(ListRecordsOutput { records, cursor })
    } else {
        bail!("Could not find repo: {repo}")
    }
}

/// GET /xrpc/com.atproto.repo.listRecords?<repo>&<collection>&<limit>&<cursor>&<rkeyStart>&<rkeyEnd>&<reverse>
#[poem::handler]
#[allow(non_snake_case)]
pub async fn list_records(
    poem::web::Query(query): poem::web::Query<ListRecordsQuery>,
    state: State<SharedState>,
) -> ApiResult<Json<ListRecordsOutput>> {
    let ListRecordsQuery {
        repo,
        collection,
        limit,
        cursor,
        rkeyStart,
        rkeyEnd,
        reverse,
    } = query;
    let limit = limit.unwrap_or(50);
    let reverse = reverse.unwrap_or(false);

    match inner_list_records(repo, collection, limit, cursor, rkeyStart, rkeyEnd, reverse, &state)
        .await
    {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ListRecordsQuery {
    pub repo: String,
    pub collection: String,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
    #[serde(rename = "rkeyStart")]
    pub rkeyStart: Option<String>,
    #[serde(rename = "rkeyEnd")]
    pub rkeyEnd: Option<String>,
    pub reverse: Option<bool>,
}
```

- [ ] **Step 4: Implement `describe_repo.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/repo/describe_repo.rs`.

```rust
// pds/src/xrpc/com/atproto/repo/describe_repo.rs
use crate::account::AccountManager;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_identity::types::DidDocument;
use rsky_lexicon::com::atproto::repo::DescribeRepoOutput;
use rsky_syntax::handle::INVALID_HANDLE;

async fn inner_describe_repo(
    repo: String,
    state: &SharedState,
) -> Result<DescribeRepoOutput> {
    let account = state.account_manager.get_account(&repo, None).await?;
    match account {
        None => bail!("Cound not find user: `{repo}`"),
        Some(account) => {
            let lock = state.id_resolver.write().await;
            let did_doc: DidDocument = match lock.did.ensure_resolve(&account.did, None).await {
                Err(err) => bail!("Could not resolve DID: `{err}`"),
                Ok(res) => res,
            };
            let handle = rsky_common::get_handle(&did_doc);
            let handle_is_correct = handle == account.handle;

            let mut actor_store = state
                .actor_store
                .read(account.did.clone(), state.blobstore.clone())
                .await?;
            let collections = actor_store.record.list_collections().await?;

            Ok(DescribeRepoOutput {
                handle: account.handle.unwrap_or(INVALID_HANDLE.to_string()),
                did: account.did,
                did_doc: serde_json::to_value(did_doc)?,
                collections,
                handle_is_correct,
            })
        }
    }
}

/// GET /xrpc/com.atproto.repo.describeRepo?<repo>
#[poem::handler]
pub async fn describe_repo(
    poem::web::Query(query): poem::web::Query<DescribeRepoQuery>,
    state: State<SharedState>,
) -> ApiResult<Json<DescribeRepoOutput>> {
    let DescribeRepoQuery { repo } = query;
    match inner_describe_repo(repo, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct DescribeRepoQuery {
    pub repo: String,
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pds --test repo_read_write_test`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add pds/src/xrpc/com/atproto/repo/delete_record.rs pds/src/xrpc/com/atproto/repo/list_records.rs pds/src/xrpc/com/atproto/repo/describe_repo.rs pds/tests/repo_read_write_test.rs
git commit -m "feat(repo): deleteRecord, listRecords, describeRepo"
```
