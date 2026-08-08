# Task 25: Admin group 1 — deleteAccount, disableAccountInvites, enableAccountInvites, disableInviteCodes, updateAccountEmail, updateAccountHandle, updateAccountPassword

**Files:**
- Create: `pds/src/xrpc/com/atproto/admin/mod.rs` (route table; replace Task 1 placeholder)
- Create: `pds/src/xrpc/com/atproto/admin/delete_account.rs`
- Create: `pds/src/xrpc/com/atproto/admin/disable_account_invites.rs`
- Create: `pds/src/xrpc/com/atproto/admin/enable_account_invites.rs`
- Create: `pds/src/xrpc/com/atproto/admin/disable_invite_codes.rs`
- Create: `pds/src/xrpc/com/atproto/admin/update_account_email.rs`
- Create: `pds/src/xrpc/com/atproto/admin/update_account_handle.rs`
- Create: `pds/src/xrpc/com/atproto/admin/update_account_password.rs`
- Test: `pds/tests/admin_account_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/admin_account_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

fn basic_auth_header() -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("admin:admin-password"))
}

#[tokio::test]
async fn update_account_password_via_admin() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.admin.updateAccountPassword")
        .header("Authorization", basic_auth_header())
        .body_json(&json!({ "did": "did:plc:alice", "password": "newpass456" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert!(state
        .account_manager
        .verify_account_password("did:plc:alice", &"newpass456".to_string())
        .await
        .unwrap());
}

#[tokio::test]
async fn admin_requires_admin_auth() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.admin.updateAccountPassword")
        .body_json(&json!({ "did": "did:plc:alice", "password": "newpass456" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn disable_invite_codes_and_enable_account_invites() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    let _ = access;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.admin.enableAccountInvites")
        .header("Authorization", basic_auth_header())
        .body_json(&json!({ "account": "did:plc:alice" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let resp = cli
        .post("/xrpc/com.atproto.admin.disableAccountInvites")
        .header("Authorization", basic_auth_header())
        .body_json(&json!({ "account": "did:plc:alice", "note": null }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}
```

Run: `cargo test -p pds --test admin_account_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement the handlers**

Each is a direct port from the matching file under `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/admin/`. Auth is the Task 2 `AdminToken` extractor (basic auth). `update_account_handle` needs the `PlcClient` + sequencer, same shape as Task 21's `update_handle`.

```rust
// pds/src/xrpc/com/atproto/admin/mod.rs
pub mod delete_account;
pub mod disable_account_invites;
pub mod disable_invite_codes;
pub mod enable_account_invites;
pub mod get_account_info;
pub mod get_account_infos;
pub mod get_invite_codes;
pub mod get_subject_status;
pub mod send_email;
pub mod update_account_email;
pub mod update_account_handle;
pub mod update_account_password;
pub mod update_subject_status;

pub fn routes() -> poem::Route {
    use poem::get;
    use poem::post;
    poem::Route::new()
        .at("/deleteAccount", post(delete_account::delete_account))
        .at("/disableAccountInvites", post(disable_account_invites::disable_account_invites))
        .at("/disableInviteCodes", post(disable_invite_codes::disable_invite_codes))
        .at("/enableAccountInvites", post(enable_account_invites::enable_account_invites))
        .at("/getAccountInfo", get(get_account_info::get_account_info))
        .at("/getAccountInfos", get(get_account_infos::get_account_infos))
        .at("/getInviteCodes", get(get_invite_codes::get_invite_codes))
        .at("/getSubjectStatus", get(get_subject_status::get_subject_status))
        .at("/sendEmail", post(send_email::send_email))
        .at("/updateAccountPassword", post(update_account_password::update_account_password))
        .at("/updateAccountEmail", post(update_account_email::update_account_email))
        .at("/updateAccountHandle", post(update_account_handle::update_account_handle))
        .at("/updateSubjectStatus", post(update_subject_status::update_subject_status))
}
```

```rust
// pds/src/xrpc/com/atproto/admin/delete_account.rs
use crate::account::helpers::account::AccountStatus;
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::DeleteAccountInput;

async fn inner_delete_account(
    body: DeleteAccountInput,
    state: &SharedState,
) -> Result<(), ApiError> {
    let DeleteAccountInput { did } = body;

    state
        .actor_store
        .destroy(&did, state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    state
        .account_manager
        .delete_account(&did)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let mut lock = state.sequencer.sequencer.write().await;
    let account_seq = lock
        .sequence_account_evt(did.clone(), AccountStatus::Deleted)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    lock.delete_all_for_user(&did, Some(vec![account_seq]))
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    Ok(())
}

/// POST /xrpc/com.atproto.admin.deleteAccount
#[poem::handler]
pub async fn delete_account(
    body: Json<DeleteAccountInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_delete_account(body.0, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/admin/disable_account_invites.rs
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::DisableAccountInvitesInput;

/// POST /xrpc/com.atproto.admin.disableAccountInvites
#[poem::handler]
pub async fn disable_account_invites(
    body: Json<DisableAccountInvitesInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    let DisableAccountInvitesInput { account, .. } = body.0;
    match state
        .account_manager
        .set_account_invites_disabled(&account, true)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/admin/enable_account_invites.rs
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::EnableAccountInvitesInput;

/// POST /xrpc/com.atproto.admin.enableAccountInvites
#[poem::handler]
pub async fn enable_account_invites(
    body: Json<EnableAccountInvitesInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    let EnableAccountInvitesInput { account, .. } = body.0;
    match state
        .account_manager
        .set_account_invites_disabled(&account, false)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/admin/disable_invite_codes.rs
use crate::account::{AccountManager, DisableInviteCodesOpts};
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::DisableInviteCodesInput;

async fn inner_disable_invite_codes(
    body: DisableInviteCodesInput,
    account_manager: &AccountManager,
) -> Result<()> {
    let DisableInviteCodesInput { codes, accounts } = body;
    let codes: Vec<String> = codes.unwrap_or_else(Vec::new);
    let accounts: Vec<String> = accounts.unwrap_or_else(Vec::new);

    if accounts.contains(&"admin".to_string()) {
        bail!("cannot disable admin invite codes")
    }

    account_manager
        .disable_invite_codes(DisableInviteCodesOpts { codes, accounts })
        .await
}

/// POST /xrpc/com.atproto.admin.disableInviteCodes
#[poem::handler]
pub async fn disable_invite_codes(
    body: Json<DisableInviteCodesInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_disable_invite_codes(body.0, &state.account_manager).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/admin/update_account_password.rs
use crate::account::{AccountManager, UpdateAccountPasswordOpts};
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::UpdateAccountPasswordInput;

/// POST /xrpc/com.atproto.admin.updateAccountPassword
#[poem::handler]
pub async fn update_account_password(
    body: Json<UpdateAccountPasswordInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    let UpdateAccountPasswordInput { did, password } = body.0;
    match state
        .account_manager
        .update_account_password(UpdateAccountPasswordOpts { did, password })
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/admin/update_account_email.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::{AccountManager, UpdateEmailOpts};
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::UpdateAccountEmailInput;

async fn inner_update_account_email(
    body: UpdateAccountEmailInput,
    account_manager: &AccountManager,
) -> Result<()> {
    let account = account_manager
        .get_account(
            &body.account,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await?;
    match account {
        None => bail!("Account does not exist: {}", body.account),
        Some(account) => {
            account_manager
                .update_email(UpdateEmailOpts {
                    did: account.did,
                    email: body.email.clone(),
                })
                .await
        }
    }
}

/// POST /xrpc/com.atproto.admin.updateAccountEmail
#[poem::handler]
pub async fn update_account_email(
    body: Json<UpdateAccountEmailInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_update_account_email(body.0, &state.account_manager).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/admin/update_account_handle.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::handle::{normalize_and_validate_handle, HandleValidationOpts};
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::admin::UpdateAccountHandleInput;

async fn inner_update_account_handle(
    body: UpdateAccountHandleInput,
    state: &SharedState,
) -> Result<()> {
    let UpdateAccountHandleInput { did, handle } = body;
    let handle = normalize_and_validate_handle(
        HandleValidationOpts {
            handle,
            did: Some(did.clone()),
            allow_reserved: None,
        },
        &state.config.identity.service_handle_domains,
    )
    .await
    .map_err(|e| anyhow::Error::msg(format!("{:?}: {}", e.kind, e.message)))?;

    let account = state
        .account_manager
        .get_account(
            &handle,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await?;

    match account {
        Some(account) if account.did != did => bail!("Handle already taken: {handle}"),
        Some(_) => (),
        None => {
            state
                .plc_client
                .update_handle(&did, &PDS_PLC_ROTATION_KEYPAIR.secret_key(), &handle)
                .await?;
            state.account_manager.update_handle(&did, &handle).await?;
        }
    }
    let mut lock = state.sequencer.sequencer.write().await;
    lock.sequence_identity_evt(did.clone(), Some(handle.clone()))
        .await?;
    Ok(())
}

/// POST /xrpc/com.atproto.admin.updateAccountHandle
#[poem::handler]
pub async fn update_account_handle(
    body: Json<UpdateAccountHandleInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_update_account_handle(body.0, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test admin_account_test`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/admin pds/tests/admin_account_test.rs
git commit -m "feat(admin): account mutation handlers"
```
