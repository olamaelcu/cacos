# Task 10: Invites — createInviteCode, createInviteCodes, getAccountInviteCodes

**Files:**
- Create: `pds/src/xrpc/com/atproto/server/create_invite_code.rs`
- Create: `pds/src/xrpc/com/atproto/server/create_invite_codes.rs`
- Create: `pds/src/xrpc/com/atproto/server/get_account_invite_codes.rs`
- Test: `pds/tests/server_invites_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/server_invites_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

fn basic_auth_header() -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("admin:admin-password"))
}

#[tokio::test]
async fn create_invite_code_via_admin() {
    let (state, _dirs) = test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.createInviteCode")
        .header("Authorization", basic_auth_header())
        .body_json(&json!({ "useCount": 1 }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["code"].as_str().is_some());
}

#[tokio::test]
async fn get_account_invite_codes_requires_full_access() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:a", "a.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.server.getAccountInviteCodes?includeUsed=false&createAvailable=false")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["codes"].as_array().unwrap().len(), 0);
}
```

Run: `cargo test -p pds --test server_invites_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement the handlers**

```rust
// pds/src/xrpc/com/atproto/server/create_invite_code.rs
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::com::atproto::server::gen_invite_code;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::{AccountCodes, CreateInviteCodeInput, CreateInviteCodeOutput};

/// POST /xrpc/com.atproto.server.createInviteCode
#[poem::handler]
pub async fn create_invite_code(
    body: Json<CreateInviteCodeInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<Json<CreateInviteCodeOutput>> {
    let CreateInviteCodeInput {
        use_count,
        for_account,
    } = body.0;
    let code = gen_invite_code();

    match state
        .account_manager
        .create_invite_codes(
            vec![AccountCodes {
                codes: vec![code.clone()],
                account: for_account.unwrap_or("admin".to_owned()),
            }],
            use_count,
        )
        .await
    {
        Ok(_) => Ok(Json(CreateInviteCodeOutput { code })),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/create_invite_codes.rs
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::com::atproto::server::gen_invite_codes;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::{
    AccountCodes, CreateInviteCodesInput, CreateInviteCodesOutput,
};

/// POST /xrpc/com.atproto.server.createInviteCodes
#[poem::handler]
pub async fn create_invite_codes(
    body: Json<CreateInviteCodesInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<Json<CreateInviteCodesOutput>> {
    let CreateInviteCodesInput {
        use_count,
        code_count,
        for_accounts,
    } = body.0;
    let for_accounts = for_accounts.unwrap_or_else(|| vec!["admin".to_owned()]);

    let mut account_codes: Vec<AccountCodes> = Vec::new();
    for account in for_accounts {
        let codes = gen_invite_codes(code_count);
        account_codes.push(AccountCodes { account, codes });
    }

    match state
        .account_manager
        .create_invite_codes(account_codes.clone(), use_count)
        .await
    {
        Ok(_) => Ok(Json(CreateInviteCodesOutput {
            codes: account_codes,
        })),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/get_account_invite_codes.rs
use crate::account::helpers::invite::CodeDetail;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::com::atproto::server::gen_invite_codes;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use chrono::NaiveDateTime;
use poem::web::Json;
use poem::State;
use rsky_common::env::{env_bool, env_int};
use rsky_common::RFC3339_VARIANT;
use rsky_lexicon::com::atproto::server::GetAccountInviteCodesOutput;
use std::time::SystemTime;

struct CalculateCodesToCreateOpts {
    pub user_created_at: usize,
    pub codes: Vec<CodeDetail>,
    pub epoch: usize,
    pub interval: usize,
}

fn calculate_codes_to_create(opts: CalculateCodesToCreateOpts) -> Result<(usize, usize), ApiError> {
    let routine_codes: Vec<CodeDetail> = opts
        .codes
        .into_iter()
        .filter(|code| code.created_by != "admin")
        .collect();
    let unused_routine_codes: Vec<CodeDetail> = routine_codes
        .clone()
        .into_iter()
        .filter(|row| !row.disabled && row.available as usize > row.uses.len())
        .collect();

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("timestamp in micros since UNIX epoch")
        .as_micros() as usize;
    let user_lifespan = now - opts.user_created_at;

    let could_create: usize = if opts.user_created_at >= opts.epoch {
        user_lifespan / opts.interval
    } else {
        let could_create_total = user_lifespan / opts.interval;
        let user_pre_epoch_lifespan = opts.epoch - opts.user_created_at;
        let could_create_before_epoch = user_pre_epoch_lifespan / opts.interval;
        could_create_total - could_create_before_epoch
    };
    let epoch_codes: Vec<CodeDetail> = routine_codes
        .clone()
        .into_iter()
        .filter(|code| {
            let datetime = NaiveDateTime::parse_from_str(&code.created_at, RFC3339_VARIANT)
                .unwrap()
                .and_utc()
                .timestamp_micros() as usize;
            datetime > opts.epoch
        })
        .collect();
    let to_create = std::cmp::min(
        5usize.saturating_sub(unused_routine_codes.len()),
        could_create.saturating_sub(epoch_codes.len()),
    );
    Ok((to_create, routine_codes.len() + to_create))
}

async fn inner_get_account_invite_codes(
    include_used: bool,
    create_available: bool,
    auth: AccessFull,
    state: &SharedState,
) -> Result<GetAccountInviteCodesOutput, ApiError> {
    let requester = auth.access.credentials.unwrap().did.unwrap();
    let account = state
        .account_manager
        .get_account(&requester, None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let mut user_codes = state
        .account_manager
        .get_account_invite_codes(&requester)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        let mut created: Vec<CodeDetail> = Vec::new();
        if create_available
            && env_bool("PDS_INVITE_REQUIRED").unwrap_or(true)
            && env_int("PDS_INVITE_INTERVAL").is_some()
        {
            let user_created_at =
                NaiveDateTime::parse_from_str(&account.created_at, RFC3339_VARIANT)
                    .map_err(|_| ApiError::RuntimeError)?
                    .and_utc()
                    .timestamp_micros() as usize;
            let (to_create, total) = calculate_codes_to_create(CalculateCodesToCreateOpts {
                user_created_at,
                codes: user_codes.clone(),
                epoch: env_int("PDS_INVITE_EPOCH").unwrap_or(0),
                interval: env_int("PDS_INVITE_INTERVAL").unwrap(),
            })?;
            if to_create > 0 {
                let codes = gen_invite_codes(to_create as i32);
                created = state
                    .account_manager
                    .create_account_invite_codes(
                        &requester,
                        codes,
                        total,
                        account.invites_disabled.unwrap_or(0) == 1,
                    )
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
            }
        }
        let mut all_codes: Vec<CodeDetail> = Vec::new();
        all_codes.append(&mut created);
        all_codes.append(&mut user_codes);
        let filtered: Vec<CodeDetail> = all_codes
            .into_iter()
            .filter(|code| {
                if code.disabled {
                    return false;
                }
                if !include_used && code.uses.len() >= code.available as usize {
                    return false;
                }
                true
            })
            .collect();
        Ok(GetAccountInviteCodesOutput { codes: filtered })
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// GET /xrpc/com.atproto.server.getAccountInviteCodes?<includeUsed>&<createAvailable>
#[poem::handler]
pub async fn get_account_invite_codes(
    poem::web::Query(query): poem::web::Query<GetAccountInviteCodesQuery>,
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<Json<GetAccountInviteCodesOutput>> {
    let GetAccountInviteCodesQuery {
        includeUsed,
        createAvailable,
    } = query;
    match inner_get_account_invite_codes(includeUsed, createAvailable, auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetAccountInviteCodesQuery {
    pub includeUsed: bool,
    pub createAvailable: bool,
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test server_invites_test`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/server/create_invite_code.rs pds/src/xrpc/com/atproto/server/create_invite_codes.rs pds/src/xrpc/com/atproto/server/get_account_invite_codes.rs pds/tests/server_invites_test.rs
git commit -m "feat(server): invite handlers"
```
