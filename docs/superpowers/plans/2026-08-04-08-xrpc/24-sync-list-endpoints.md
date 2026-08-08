# Task 24: Sync list endpoints — listBlobs, listRepos, getRepoStatus

**Files:**
- Create: `pds/src/xrpc/com/atproto/sync/list_blobs.rs`
- Create: `pds/src/xrpc/com/atproto/sync/list_repos.rs`
- Create: `pds/src/xrpc/com/atproto/sync/get_repo_status.rs`
- Test: `pds/tests/sync_list_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/sync_list_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;

#[tokio::test]
async fn list_repos_lists_local_accounts() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.sync.listRepos")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let repos = body["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 2);
    assert!(repos.iter().all(|r| r["active"] == true));
}

#[tokio::test]
async fn list_blobs_empty_for_fresh_account() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.sync.listBlobs?did=did:plc:alice")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["cids"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_repo_status_active() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.sync.getRepoStatus?did=did:plc:alice")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["active"], true);
}
```

Run: `cargo test -p pds --test sync_list_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `list_blobs.rs` and `get_repo_status.rs`**

```rust
// pds/src/xrpc/com/atproto/sync/list_blobs.rs
use crate::actor_store::blob::ListBlobsOpts;
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::OptionalAccessOrAdminToken;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::sync::ListBlobsOutput;

async fn inner_list_blobs(
    did: String,
    since: Option<String>,
    limit: Option<u16>,
    cursor: Option<String>,
    auth: OptionalAccessOrAdminToken,
    state: &SharedState,
) -> Result<ListBlobsOutput> {
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
    let blob_cids = actor_store
        .blob
        .list_blobs(ListBlobsOpts {
            since,
            cursor,
            limit: limit.unwrap_or(500),
        })
        .await?;

    let last_blob: Option<String> = blob_cids.last().cloned();
    Ok(ListBlobsOutput {
        cursor: last_blob,
        cids: blob_cids,
    })
}

/// GET /xrpc/com.atproto.sync.listBlobs?<did>&<since>&<limit>&<cursor>
#[poem::handler]
pub async fn list_blobs(
    poem::web::Query(query): poem::web::Query<ListBlobsQuery>,
    auth: OptionalAccessOrAdminToken,
    state: State<SharedState>,
) -> ApiResult<Json<ListBlobsOutput>> {
    let ListBlobsQuery {
        did,
        since,
        limit,
        cursor,
    } = query;
    match inner_list_blobs(did, since, limit, cursor, auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ListBlobsQuery {
    pub did: String,
    pub since: Option<String>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}
```

```rust
// pds/src/xrpc/com/atproto/sync/get_repo_status.rs
use crate::account::helpers::account::{format_account_status, AccountStatus, FormattedAccountStatus};
use crate::account::AccountManager;
use crate::xrpc::com::atproto::repo::assert_repo_availability;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::sync::{GetRepoStatusOutput, RepoStatus};

async fn inner_get_repo(
    did: String,
    state: &SharedState,
) -> Result<GetRepoStatusOutput> {
    let account = assert_repo_availability(&did, true, &state.account_manager).await?;
    let FormattedAccountStatus { active, status } = format_account_status(Some(account));

    let mut rev: Option<String> = None;
    if active {
        let actor_store = state
            .actor_store
            .read(did.clone(), state.blobstore.clone())
            .await?;
        let storage_guard = actor_store.storage.read().await;
        let root = storage_guard.get_root_detailed().await?;
        rev = Some(root.rev);
    }

    Ok(GetRepoStatusOutput {
        did,
        active,
        status: match status {
            None => None,
            Some(status) => match status {
                AccountStatus::Active => None,
                AccountStatus::Takendown => Some(RepoStatus::Takedown),
                AccountStatus::Suspended => Some(RepoStatus::Suspended),
                AccountStatus::Deleted => None,
                AccountStatus::Deactivated => Some(RepoStatus::Deactivated),
                AccountStatus::Desynchronized => Some(RepoStatus::Desynchronized),
                AccountStatus::Throttled => Some(RepoStatus::Throttled),
            },
        },
        rev,
    })
}

/// GET /xrpc/com.atproto.sync.getRepoStatus?<did>
#[poem::handler]
pub async fn get_repo_status(
    poem::web::Query(query): poem::web::Query<GetRepoStatusQuery>,
    state: State<SharedState>,
) -> ApiResult<Json<GetRepoStatusOutput>> {
    let GetRepoStatusQuery { did } = query;
    match inner_get_repo(did, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetRepoStatusQuery {
    pub did: String,
}
```

- [ ] **Step 3: Implement `list_repos.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/sync/list_repos.rs` (direct SQL over the account DB via the Plan 05 `Db`).

```rust
// pds/src/xrpc/com/atproto/sync/list_repos.rs
use crate::account::helpers::account::{
    format_account_status, AccountStatus, ActorAccount, FormattedAccountStatus,
};
use crate::account::AccountManager;
use crate::db::pagination::{SortDirection, TimeCidKeyset};
use crate::xrpc::{ApiError, ApiResult};
use anyhow::Result;
use poem::web::Json;
use rsky_lexicon::com::atproto::sync::{ListReposOutput, RefRepo as LexiconRepo, RepoStatus};
use rusqlite::params_from_iter;
use rusqlite::types::Value as SqlValue;

#[derive(Debug, Clone)]
pub struct RepoRow {
    pub did: String,
    pub cid: String,
    pub rev: String,
    pub created_at: String,
    pub deactivated_at: Option<String>,
    pub takedown_ref: Option<String>,
}

pub async fn paginate_repos(db: &crate::db::Db, limit: i64, cursor: Option<String>) -> Result<Vec<RepoRow>> {
    let keyset = TimeCidKeyset::new("actor.\"createdAt\"", "actor.did");
    let unpacked = keyset.unpack(cursor.as_deref())?;

    let mut sql = "\
        SELECT actor.did, repo_root.cid, repo_root.rev, actor.\"createdAt\", \
        actor.\"deactivatedAt\", actor.\"takedownRef\" \
        FROM actor JOIN repo_root ON repo_root.did = actor.did"
        .to_string();
    let mut sql_params: Vec<SqlValue> = Vec::new();
    if let Some((created_at, did)) = unpacked {
        sql.push_str(&format!(" WHERE {}", keyset.where_clause(SortDirection::Asc)));
        sql_params.push(SqlValue::Text(created_at));
        sql_params.push(SqlValue::Text(did));
    }
    sql.push_str(&format!(
        " ORDER BY {} LIMIT {limit}",
        keyset.order_by_clause(SortDirection::Asc)
    ));

    db.run(move |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(sql_params.iter()), |row| {
                Ok(RepoRow {
                    did: row.get(0)?,
                    cid: row.get(1)?,
                    rev: row.get(2)?,
                    created_at: row.get(3)?,
                    deactivated_at: row.get(4)?,
                    takedown_ref: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<RepoRow>, rusqlite::Error>>()?;
        Ok(rows)
    })
    .await
}

async fn inner_list_repos(
    limit: Option<i64>,
    cursor: Option<String>,
    db: &crate::db::Db,
) -> Result<ListReposOutput> {
    let keyset = TimeCidKeyset::new("actor.\"createdAt\"", "actor.did");
    let result = paginate_repos(db, limit.unwrap_or(500), cursor).await?;
    let cursor_rows = result
        .iter()
        .map(|row| (row.created_at.clone(), row.did.clone()))
        .collect::<Vec<(String, String)>>();
    let repos = result
        .into_iter()
        .map(|row| {
            let FormattedAccountStatus { active, status } =
                format_account_status(Some(ActorAccount {
                    did: row.did.clone(),
                    handle: None,
                    created_at: row.created_at,
                    takedown_ref: row.takedown_ref,
                    deactivated_at: row.deactivated_at,
                    delete_after: None,
                    email: None,
                    invites_disabled: None,
                    email_confirmed_at: None,
                }));
            LexiconRepo {
                did: row.did,
                head: row.cid,
                rev: row.rev,
                active: Some(active),
                status: match status {
                    None => None,
                    Some(status) => match status {
                        AccountStatus::Active => None,
                        AccountStatus::Takendown => Some(RepoStatus::Takedown),
                        AccountStatus::Suspended => Some(RepoStatus::Suspended),
                        AccountStatus::Deleted => None,
                        AccountStatus::Deactivated => Some(RepoStatus::Deactivated),
                        AccountStatus::Desynchronized => Some(RepoStatus::Desynchronized),
                        AccountStatus::Throttled => Some(RepoStatus::Throttled),
                    },
                },
            }
        })
        .collect::<Vec<LexiconRepo>>();
    Ok(ListReposOutput {
        cursor: keyset.pack_from_result(&cursor_rows)?,
        repos,
    })
}

/// GET /xrpc/com.atproto.sync.listRepos?<limit>&<cursor>
#[poem::handler]
pub async fn list_repos(
    poem::web::Query(query): poem::web::Query<ListReposQuery>,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<Json<ListReposOutput>> {
    let ListReposQuery { limit, cursor } = query;
    match inner_list_repos(limit, cursor, &state.account_manager.db).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ListReposQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}
```

> **Plan 01 note:** `crate::db::{Db, pagination::{TimeCidKeyset, SortDirection}}` come from Plan 01's db module; `AccountManager.db` field from Plan 05.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pds --test sync_list_test`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add pds/src/xrpc/com/atproto/sync/list_blobs.rs pds/src/xrpc/com/atproto/sync/list_repos.rs pds/src/xrpc/com/atproto/sync/get_repo_status.rs pds/tests/sync_list_test.rs
git commit -m "feat(sync): listBlobs, listRepos, getRepoStatus"
```
