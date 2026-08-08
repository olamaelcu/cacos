# Task 26: Admin group 2 + temp — getAccountInfo(s), getInviteCodes, getSubjectStatus, updateSubjectStatus, sendEmail, checkSignupQueue

**Files:**
- Create: `pds/src/xrpc/com/atproto/admin/get_account_info.rs`
- Create: `pds/src/xrpc/com/atproto/admin/get_account_infos.rs`
- Create: `pds/src/xrpc/com/atproto/admin/get_invite_codes.rs`
- Create: `pds/src/xrpc/com/atproto/admin/get_subject_status.rs`
- Create: `pds/src/xrpc/com/atproto/admin/update_subject_status.rs`
- Create: `pds/src/xrpc/com/atproto/admin/send_email.rs`
- Create: `pds/src/xrpc/com/atproto/temp/mod.rs` (route table)
- Create: `pds/src/xrpc/com/atproto/temp/check_signup_queue.rs`
- Test: `pds/tests/admin_query_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/admin_query_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

fn basic_auth_header() -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("admin:admin-password"))
}

#[tokio::test]
async fn get_account_info_returns_view() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.admin.getAccountInfo?did=did:plc:alice")
        .header("Authorization", basic_auth_header())
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:alice");
    assert_eq!(body["handle"], "alice.test");
}

#[tokio::test]
async fn get_account_infos_skips_missing() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.admin.getAccountInfos?dids=did:plc:alice&dids=did:plc:missing")
        .header("Authorization", basic_auth_header())
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["infos"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn check_signup_queue_always_activated() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.temp.checkSignupQueue")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["activated"], true);
}

#[tokio::test]
async fn get_invite_codes_returns_paginated() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    // seed a couple of invite codes directly
    state
        .account_manager
        .create_invite_codes(
            vec![pds::account::AccountCodes {
                account: "did:plc:alice".to_owned(),
                codes: vec!["code-a".to_owned(), "code-b".to_owned()],
            }],
            1,
        )
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.admin.getInviteCodes?sort=recent&limit=10")
        .header("Authorization", basic_auth_header())
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["codes"].as_array().unwrap().len(), 2);
}
```

> **Test note:** `getAccountInfo`/`getAccountInfos` use the `Moderator` guard; with `PDS_MOD_SERVICE_DID` unset the bearer branch fails and only the admin-token (basic) branch succeeds. `updateSubjectStatus`/`getSubjectStatus`/`sendEmail` follow the same shapes and are covered by the same test file pattern (add tests for the takedown toggle via `updateSubjectStatus` and `sendEmail` returning `sent: true`).

Run: `cargo test -p pds --test admin_query_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `get_account_info.rs` + `get_account_infos.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/admin/get_account_info.rs` and `get_account_infos.rs`. The `Moderator` guard is used; in this port `Moderator` accepts admin tokens (Task 2) and the mod-service bearer branch is stubbed until `verify_user_did_token` supports the labeler — acceptable because `PDS_MOD_SERVICE_DID` is unset in default deployments.

```rust
// pds/src/xrpc/com/atproto/admin/get_account_info.rs
use crate::account::helpers::account::{ActorAccount, AvailabilityFlags};
use crate::account::helpers::invite::CodeDetail;
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::Moderator;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use futures::try_join;
use poem::web::Json;
use rsky_lexicon::com::atproto::admin::AccountView;
use rsky_syntax::handle::INVALID_HANDLE;
use std::collections::BTreeMap;

pub fn manages_own_invites() -> bool {
    crate::config::env_str("PDS_ENTRYWAY_URL").is_none()
}

pub fn format_account_view(
    account: ActorAccount,
    invites: Vec<CodeDetail>,
    invited_by: &BTreeMap<String, CodeDetail>,
    manages_own_invites: bool,
) -> AccountView {
    AccountView {
        did: account.did.clone(),
        handle: account.handle.unwrap_or(INVALID_HANDLE.to_string()),
        email: account.email,
        indexed_at: account.created_at,
        email_confirmed_at: account.email_confirmed_at,
        invited_by: match invited_by.get(&account.did) {
            Some(code_detail) if manages_own_invites => Some(code_detail.clone()),
            _ => None,
        },
        invites: if manages_own_invites {
            Some(invites)
        } else {
            None
        },
        invites_disabled: if manages_own_invites {
            Some(account.invites_disabled == Some(1))
        } else {
            None
        },
        related_records: None,
        invite_note: None,
    }
}

async fn inner_get_account_info(
    did: String,
    state: &SharedState,
) -> Result<AccountView> {
    let (account, invites, invited_by) = try_join!(
        state.account_manager.get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true)
            })
        ),
        state.account_manager.get_account_invite_codes(&did),
        state.account_manager.get_invited_by_for_accounts(vec![did.clone()])
    )?;
    if let Some(account) = account {
        Ok(format_account_view(
            account,
            invites,
            &invited_by,
            manages_own_invites(),
        ))
    } else {
        bail!("Account not found")
    }
}

/// GET /xrpc/com.atproto.admin.getAccountInfo?<did>
#[poem::handler]
pub async fn get_account_info(
    poem::web::Query(query): poem::web::Query<GetAccountInfoQuery>,
    _auth: Moderator,
    state: poem::State<SharedState>,
) -> ApiResult<Json<AccountView>> {
    let GetAccountInfoQuery { did } = query;
    match inner_get_account_info(did, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetAccountInfoQuery {
    pub did: String,
}
```

```rust
// pds/src/xrpc/com/atproto/admin/get_account_infos.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::xrpc::auth_extractors::Moderator;
use crate::xrpc::com::atproto::admin::get_account_info::{format_account_view, manages_own_invites};
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use poem::web::Json;
use rsky_lexicon::com::atproto::admin::{AccountView, GetAccountInfosOutput};

async fn inner_get_account_infos(
    dids: Vec<String>,
    state: &SharedState,
) -> Result<GetAccountInfosOutput> {
    let invited_by = state
        .account_manager
        .get_invited_by_for_accounts(dids.clone())
        .await?;
    let manages_own_invites = manages_own_invites();
    let mut infos: Vec<AccountView> = Vec::with_capacity(dids.len());
    for did in dids {
        let account = state
            .account_manager
            .get_account(
                &did,
                Some(AvailabilityFlags {
                    include_deactivated: Some(true),
                    include_taken_down: Some(true),
                }),
            )
            .await?;
        let Some(account) = account else { continue };
        let invites = state.account_manager.get_account_invite_codes(&did).await?;
        infos.push(format_account_view(
            account,
            invites,
            &invited_by,
            manages_own_invites,
        ));
    }
    Ok(GetAccountInfosOutput { infos })
}

/// GET /xrpc/com.atproto.admin.getAccountInfos?<dids>
#[poem::handler]
pub async fn get_account_infos(
    poem::web::Query(query): poem::web::Query<GetAccountInfosQuery>,
    _auth: Moderator,
    state: poem::State<SharedState>,
) -> ApiResult<Json<GetAccountInfosOutput>> {
    let GetAccountInfosQuery { dids } = query;
    match inner_get_account_infos(dids, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetAccountInfosQuery {
    pub dids: Vec<String>,
}
```

- [ ] **Step 3: Implement `get_invite_codes.rs`, `get_subject_status.rs`, `update_subject_status.rs`, `send_email.rs`**

```rust
// pds/src/xrpc/com/atproto/admin/get_invite_codes.rs
use crate::account::helpers::invite::{get_invite_codes_uses_v2, CodeDetail};
use crate::db::pagination::{pack_cursor, unpack_cursor, Cursor, SortDirection, TimeCidKeyset};
use crate::xrpc::auth_extractors::Moderator;
use crate::xrpc::{ApiError, ApiResult};
use anyhow::{bail, Result};
use poem::web::Json;
use rsky_lexicon::com::atproto::admin::GetInviteCodesOutput;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Row};
use std::mem;

const SELECT_CODES_WITH_USES: &str = "\
    SELECT * FROM (\
    SELECT code, \"availableUses\", disabled, \"forAccount\", \"createdBy\", \"createdAt\", \
    (SELECT count(*) FROM invite_code_use WHERE invite_code_use.code = invite_code.code) AS uses \
    FROM invite_code) codes";

#[derive(Debug, Clone)]
struct InviteCodeRow {
    code: crate::account::helpers::invite::CodeDetail,
    uses: i64,
}

fn invite_code_row(row: &Row) -> Result<InviteCodeRow, rusqlite::Error> {
    Ok(InviteCodeRow {
        code: CodeDetail {
            code: row.get(0)?,
            available: row.get(1)?,
            disabled: row.get(2)?,
            for_account: row.get(3)?,
            created_by: row.get(4)?,
            created_at: row.get(5)?,
            uses: Vec::new(),
        },
        uses: row.get(6)?,
    })
}

async fn query_codes(
    db: &crate::db::Db,
    where_clause: Option<(String, Vec<SqlValue>)>,
    order_by: String,
    limit: i64,
) -> Result<Vec<InviteCodeRow>> {
    let mut sql = SELECT_CODES_WITH_USES.to_string();
    let mut sql_params: Vec<SqlValue> = Vec::new();
    if let Some((clause, params)) = where_clause {
        sql.push_str(&format!(" WHERE {clause}"));
        sql_params.extend(params);
    }
    sql.push_str(&format!(" ORDER BY {order_by} LIMIT {limit}"));
    db.run(move |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(sql_params.iter()), invite_code_row)?
            .collect::<Result<Vec<InviteCodeRow>, rusqlite::Error>>()?;
        Ok(rows)
    })
    .await
}

async fn paginate_by_recent(
    db: &crate::db::Db,
    limit: i64,
    cursor: Option<String>,
) -> Result<(Vec<InviteCodeRow>, Option<String>)> {
    let keyset = TimeCidKeyset::new("\"createdAt\"", "code");
    let where_clause = keyset.unpack(cursor.as_deref())?.map(|(created_at, code)| {
        (
            keyset.where_clause(SortDirection::Desc),
            vec![SqlValue::Text(created_at), SqlValue::Text(code)],
        )
    });
    let rows = query_codes(
        db,
        where_clause,
        keyset.order_by_clause(SortDirection::Desc),
        limit,
    )
    .await?;
    let cursor_rows = rows
        .iter()
        .map(|row| (row.code.created_at.clone(), row.code.code.clone()))
        .collect::<Vec<(String, String)>>();
    let result_cursor = keyset.pack_from_result(&cursor_rows)?;
    Ok((rows, result_cursor))
}

async fn paginate_by_usage(
    db: &crate::db::Db,
    limit: i64,
    cursor: Option<String>,
) -> Result<(Vec<InviteCodeRow>, Option<String>)> {
    let where_clause = match unpack_cursor(cursor.as_deref())? {
        None => None,
        Some(cursor) => {
            let Ok(uses) = cursor.primary.parse::<i64>() else {
                bail!("Malformed cursor")
            };
            Some((
                "((uses, code) < (?, ?))".to_string(),
                vec![SqlValue::Integer(uses), SqlValue::Text(cursor.secondary)],
            ))
        }
    };
    let rows = query_codes(db, where_clause, "uses DESC, code DESC".to_string(), limit).await?;
    let result_cursor = pack_cursor(rows.last().map(|row| Cursor {
        primary: row.uses.to_string(),
        secondary: row.code.code.clone(),
    }));
    Ok((rows, result_cursor))
}

async fn inner_get_invite_codes(
    sort: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
    db: &crate::db::Db,
) -> Result<GetInviteCodesOutput> {
    let limit = limit.unwrap_or(100);
    let (rows, result_cursor) = match sort.as_deref() {
        Some("recent") | None => paginate_by_recent(db, limit, cursor).await?,
        Some("usage") => paginate_by_usage(db, limit, cursor).await?,
        _ => bail!("Unknown sort method: {:?}", sort),
    };

    let codes: Vec<String> = rows.iter().map(|row| row.code.code.clone()).collect();
    let mut uses = get_invite_codes_uses_v2(codes, db).await?;
    let codes = rows
        .into_iter()
        .map(|row| CodeDetail {
            code: row.code.code.clone(),
            available: row.code.available,
            disabled: row.code.disabled,
            for_account: row.code.for_account,
            created_by: row.code.created_by,
            created_at: row.code.created_at,
            uses: mem::take(uses.get_mut(&row.code.code).unwrap_or(&mut Vec::new())),
        })
        .collect::<Vec<CodeDetail>>();

    Ok(GetInviteCodesOutput {
        cursor: result_cursor,
        codes,
    })
}

/// GET /xrpc/com.atproto.admin.getInviteCodes?<sort>&<limit>&<cursor>
#[poem::handler]
pub async fn get_invite_codes(
    poem::web::Query(query): poem::web::Query<GetInviteCodesQuery>,
    _auth: Moderator,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<Json<GetInviteCodesOutput>> {
    let GetInviteCodesQuery { sort, limit, cursor } = query;
    match inner_get_invite_codes(sort, limit, cursor, &state.account_manager.db).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetInviteCodesQuery {
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}
```

> **Plan 05 note:** `CodeDetail` fields (`code, available, disabled, for_account, created_by, created_at, uses`) are assumed from Plan 05's `helpers::invite`. If Plan 05 named fields differently, adjust the row mapper and `inner_get_invite_codes` accordingly.

```rust
// pds/src/xrpc/com/atproto/admin/get_subject_status.rs
use crate::xrpc::auth_extractors::Moderator;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use futures::try_join;
use lexicon_cid::Cid;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::{RepoBlobRef, RepoRef, Subject, SubjectStatus};
use rsky_lexicon::com::atproto::repo::StrongRef;
use std::str::FromStr;

async fn inner_get_subject_status(
    did: Option<String>,
    uri: Option<String>,
    blob: Option<String>,
    state: &SharedState,
) -> Result<SubjectStatus> {
    let mut body: Option<SubjectStatus> = None;
    if let Some(blob) = blob {
        match did {
            None => bail!("Must provide a did to request blob state"),
            Some(did) => {
                let actor_store = state
                    .actor_store
                    .read(did.clone(), state.blobstore.clone())
                    .await?;

                let takedown = actor_store
                    .blob
                    .get_blob_takedown_status(Cid::from_str(&blob)?)
                    .await?;
                if let Some(takedown) = takedown {
                    body = Some(SubjectStatus {
                        subject: Subject::RepoBlobRef(RepoBlobRef {
                            did,
                            cid: blob,
                            record_uri: None,
                        }),
                        takedown: Some(takedown),
                        deactivated: None,
                    });
                }
            }
        }
    } else if let Some(uri) = uri {
        let uri_without_prefix = uri.replace("at://", "");
        let parts = uri_without_prefix.split('/').collect::<Vec<&str>>();
        if let (Some(uri_hostname), Some(_), Some(_)) = (parts.first(), parts.get(1), parts.get(2))
        {
            let actor_store = state
                .actor_store
                .read(
                    uri_hostname.to_string(),
                    state.blobstore.clone(),
                )
                .await?;
            let (takedown, cid) = try_join!(
                actor_store.record.get_record_takedown_status(uri.clone()),
                actor_store.record.get_current_record_cid(uri.clone()),
            )?;
            if let (Some(cid), Some(takedown)) = (cid, takedown) {
                body = Some(SubjectStatus {
                    subject: Subject::StrongRef(StrongRef {
                        uri,
                        cid: cid.to_string(),
                    }),
                    takedown: Some(takedown),
                    deactivated: None,
                });
            }
        }
    } else if let Some(did) = did {
        let status = state.account_manager.get_account_admin_status(&did).await?;
        if let Some(status) = status {
            body = Some(SubjectStatus {
                subject: Subject::RepoRef(RepoRef { did }),
                takedown: Some(status.takedown),
                deactivated: Some(status.deactivated),
            });
        }
    } else {
        bail!("No provided subject");
    }
    match body {
        None => bail!("NotFound: Subject not found"),
        Some(body) => Ok(body),
    }
}

/// GET /xrpc/com.atproto.admin.getSubjectStatus?<did>&<uri>&<blob>
#[poem::handler]
pub async fn get_subject_status(
    poem::web::Query(query): poem::web::Query<GetSubjectStatusQuery>,
    _auth: Moderator,
    state: State<SharedState>,
) -> ApiResult<Json<SubjectStatus>> {
    let GetSubjectStatusQuery { did, uri, blob } = query;
    match inner_get_subject_status(did, uri, blob, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetSubjectStatusQuery {
    pub did: Option<String>,
    pub uri: Option<String>,
    pub blob: Option<String>,
}
```

```rust
// pds/src/xrpc/com/atproto/admin/update_subject_status.rs
use crate::xrpc::auth_extractors::Moderator;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::Result;
use lexicon_cid::Cid;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::{Subject, SubjectStatus, UpdateSubjectStatusOutput};
use rsky_syntax::aturi::AtUri;
use std::str::FromStr;

async fn inner_update_subject_status(
    body: SubjectStatus,
    state: &SharedState,
) -> Result<UpdateSubjectStatusOutput> {
    let SubjectStatus {
        subject,
        takedown,
        deactivated,
    } = body;

    if let Some(takedown) = &takedown {
        match &subject {
            Subject::RepoRef(subject) => {
                state
                    .account_manager
                    .takedown_account(&subject.did, takedown.clone())
                    .await?;
            }
            Subject::StrongRef(subject) => {
                let subject_at_uri: AtUri = subject.uri.clone().try_into()?;
                let did = subject_at_uri.get_hostname().to_string();
                let actor_store = state
                    .actor_store
                    .transact(did.clone(), state.blobstore.clone())
                    .await?;
                actor_store
                    .record
                    .update_record_takedown_status(&subject_at_uri, takedown.clone())
                    .await?;
            }
            Subject::RepoBlobRef(subject) => {
                let actor_store = state
                    .actor_store
                    .transact(
                        subject.did.clone(),
                        state.blobstore.clone(),
                    )
                    .await?;
                actor_store
                    .blob
                    .update_blob_takedown_status(Cid::from_str(&subject.cid)?, takedown.clone())
                    .await?;
            }
        }
    }

    if let Some(deactivated) = deactivated {
        if let Subject::RepoRef(subject) = &subject {
            if deactivated.applied {
                state
                    .account_manager
                    .deactivate_account(&subject.did, None)
                    .await?;
            } else {
                state.account_manager.activate_account(&subject.did).await?;
            }
        }
    }

    if let Subject::RepoRef(subject) = &subject {
        let status = state.account_manager.get_account_status(&subject.did).await?;
        let mut lock = state.sequencer.sequencer.write().await;
        lock.sequence_account_evt(subject.did.clone(), status)
            .await?;
    }

    Ok(UpdateSubjectStatusOutput { subject, takedown })
}

/// POST /xrpc/com.atproto.admin.updateSubjectStatus
#[poem::handler]
pub async fn update_subject_status(
    body: Json<SubjectStatus>,
    _auth: Moderator,
    state: State<SharedState>,
) -> ApiResult<Json<UpdateSubjectStatusOutput>> {
    match inner_update_subject_status(body.0, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/admin/send_email.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::mailer::moderation::{HtmlMailOpts, ModerationMailer};
use crate::xrpc::auth_extractors::Moderator;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::{SendMailInput, SendMailOutput};

async fn inner_send_email(
    body: SendMailInput,
    state: &SharedState,
) -> Result<SendMailOutput> {
    let SendMailInput {
        content,
        recipient_did,
        subject,
        ..
    } = body;
    let subject = subject.unwrap_or("Message via your PDS".to_string());

    let account = state
        .account_manager
        .get_account(
            &recipient_did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await?;

    match account {
        None => bail!("Recipient not found"),
        Some(account) => match account.email {
            None => bail!("account does not have an email address"),
            Some(email) => {
                ModerationMailer::send_html(HtmlMailOpts {
                    to: email,
                    subject,
                    html: content,
                })
                .await?;

                Ok(SendMailOutput { sent: true })
            }
        },
    }
}

/// POST /xrpc/com.atproto.admin.sendEmail
#[poem::handler]
pub async fn send_email(
    body: Json<SendMailInput>,
    _auth: Moderator,
    state: State<SharedState>,
) -> ApiResult<Json<SendMailOutput>> {
    match inner_send_email(body.0, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 4: Implement `temp/check_signup_queue.rs` + temp route table**

```rust
// pds/src/xrpc/com/atproto/temp/check_signup_queue.rs
use crate::xrpc::auth_extractors::AccessStandardSignupQueued;
use crate::xrpc::ApiResult;
use poem::web::Json;
use rsky_lexicon::com::atproto::temp::CheckSignupQueueOutput;

/// GET /xrpc/com.atproto.temp.checkSignupQueue
#[poem::handler]
pub async fn check_signup_queue(
    _auth: AccessStandardSignupQueued,
) -> ApiResult<Json<CheckSignupQueueOutput>> {
    Ok(Json(CheckSignupQueueOutput {
        activated: true,
        place_in_queue: None,
        estimated_time_ms: None,
    }))
}
```

```rust
// pds/src/xrpc/com/atproto/temp/mod.rs
pub mod check_signup_queue;

pub fn routes() -> poem::Route {
    use poem::get;
    poem::Route::new().at("/checkSignupQueue", get(check_signup_queue::check_signup_queue))
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pds --test admin_query_test`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add pds/src/xrpc/com/atproto/admin pds/src/xrpc/com/atproto/temp pds/tests/admin_query_test.rs
git commit -m "feat(admin,temp): query/status handlers and checkSignupQueue"
```
