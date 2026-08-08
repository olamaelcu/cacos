# Task 7: Account lifecycle — createAccount, activateAccount, deactivateAccount, deleteAccount, checkAccountStatus

**Files:**
- Create: `pds/src/xrpc/com/atproto/server/create_account.rs`
- Create: `pds/src/xrpc/com/atproto/server/activate_account.rs`
- Create: `pds/src/xrpc/com/atproto/server/deactivate_account.rs`
- Create: `pds/src/xrpc/com/atproto/server/delete_account.rs`
- Create: `pds/src/xrpc/com/atproto/server/check_account_status.rs`
- Test: `pds/tests/server_account_lifecycle_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/server_account_lifecycle_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn create_account_success() {
    let (state, _dirs) = test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.createAccount")
        .body_json(&json!({
            "handle": "newbie.test",
            "email": "newbie@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["handle"], "newbie.test");
    assert!(body["did"].as_str().unwrap().starts_with("did:plc:"));
    assert!(body["accessJwt"].as_str().is_some());
}

#[tokio::test]
async fn create_account_requires_email() {
    let (state, _dirs) = test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.createAccount")
        .body_json(&json!({ "handle": "noemail.test", "password": "password123" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidEmail");
}

#[tokio::test]
async fn duplicate_handle_rejected() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:dup", "dup.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.createAccount")
        .body_json(&json!({
            "handle": "dup.test",
            "email": "dup2@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "HandleNotAvailable");
}

#[tokio::test]
async fn deactivate_then_activate_account() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:d", "d.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.deactivateAccount")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "deleteAfter": null }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let resp = cli
        .post("/xrpc/com.atproto.server.activateAccount")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let resp = cli
        .get("/xrpc/com.atproto.server.checkAccountStatus")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["activated"], true);
}
```

> `activateAccount` and `checkAccountStatus` call `assert_valid_did_documents_for_service`, which hits the PLC network. To keep these tests hermetic, `test_state()` provides `MockPlcClient` (Task 18) whose `get_document_data` returns a document whose `atproto_pds` endpoint matches the test public URL and whose verification method is the PDS signing key. The mock is defined to satisfy these assertions; implement it in Task 18 but scaffold it now in `pds/src/plc/mod.rs` so Task 7's tests run.

Run: `cargo test -p pds --test server_account_lifecycle_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `create_account.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/create_account.rs`. The PLC-op flow (`format_did_and_plc_op`) is kept, but the generated op is submitted through the injected `PlcClient` instead of a directly-constructed `plc::Client`, and account creation runs through the shared state.

```rust
// pds/src/xrpc/com/atproto/server/create_account.rs
use crate::account::helpers::account::AccountStatus;
use crate::account::{AccountManager, CreateAccountOpts};
use crate::actor_store::ActorStore;
use crate::context::PDS_REPO_SIGNING_KEYPAIR;
use crate::handle::{normalize_and_validate_handle, HandleValidationOpts};
use crate::plc::operations::{create_op, CreateAtprotoOpInput};
use crate::plc::types::{OpOrTombstone, Operation};
use crate::sequencer::events::sync_evt_data_from_commit;
use crate::xrpc::auth_extractors::UserDidAuthOptional;
use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
use crate::xrpc::types::SharedIdResolver;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use email_address::EmailAddress;
use poem::web::Json;
use poem::State;
use rsky_crypto::utils::encode_did_key;
use rsky_lexicon::com::atproto::server::{CreateAccountInput, CreateAccountOutput};
use std::env;

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct TransformedCreateAccountInput {
    pub email: String,
    pub handle: String,
    pub did: String,
    pub invite_code: Option<String>,
    pub password: String,
    pub plc_op: Option<Operation>,
    pub deactivated: bool,
}

/// POST /xrpc/com.atproto.server.createAccount
#[allow(clippy::too_many_arguments)]
#[poem::handler]
pub async fn server_create_account(
    body: Json<CreateAccountInput>,
    auth: UserDidAuthOptional,
    state: State<SharedState>,
) -> ApiResult<Json<CreateAccountOutput>> {
    tracing::info!("Creating new user account");
    let requester = match auth.access {
        Some(access) if access.credentials.is_some() => access.credentials.unwrap().iss,
        _ => None,
    };
    let TransformedCreateAccountInput {
        email,
        handle,
        did,
        invite_code,
        password,
        deactivated,
        plc_op,
    } = validate_inputs_for_local_pds(&state, body.0, requester).await?;

    // Create new actor repo
    if let Err(error) = state
        .actor_store
        .create(&did, &PDS_REPO_SIGNING_KEYPAIR)
        .await
    {
        tracing::error!("Failed to create actor store\n{error:?}");
        return Err(ApiError::RuntimeError);
    }
    let commit = {
        let actor_txn = match state
            .actor_store
            .transact(did.clone(), state.blobstore.clone())
            .await
        {
            Ok(actor_txn) => actor_txn,
            Err(error) => {
                tracing::error!("Failed to open actor store\n{error:?}");
                state.actor_store.destroy(&did, state.blobstore.clone()).await.map_err(|_| ApiError::RuntimeError)?;
                return Err(ApiError::RuntimeError);
            }
        };
        match actor_txn.create_repo(Vec::new()).await {
            Ok(commit) => commit,
            Err(error) => {
                tracing::error!("Failed to create repo\n{error:?}");
                state.actor_store.destroy(&did, state.blobstore.clone()).await.map_err(|_| ApiError::RuntimeError)?;
                return Err(ApiError::RuntimeError);
            }
        }
    };

    // Generate a real did with PLC
    match plc_op {
        None => {}
        Some(op) => {
            match state
                .plc_client
                .send_operation(&did, &OpOrTombstone::Operation(op))
                .await
            {
                Ok(_) => tracing::info!("Successfully sent PLC Operation"),
                Err(_) => {
                    tracing::error!("Failed to create did:plc");
                    state.actor_store.destroy(&did, state.blobstore.clone()).await.map_err(|_| ApiError::RuntimeError)?;
                    return Err(ApiError::RuntimeError);
                }
            }
        }
    }

    let did_doc = match safe_resolve_did_doc(&state.id_resolver, &did, Some(true)).await {
        Ok(res) => res,
        Err(error) => {
            tracing::error!("Error resolving DID Doc\n{error}");
            state.actor_store.destroy(&did, state.blobstore.clone()).await.map_err(|_| ApiError::RuntimeError)?;
            return Err(ApiError::RuntimeError);
        }
    };

    let (access_jwt, refresh_jwt);
    match state
        .account_manager
        .create_account(CreateAccountOpts {
            did: did.clone(),
            handle: handle.clone(),
            email: Some(email),
            password: Some(password),
            repo_cid: commit.commit_data.cid,
            repo_rev: commit.commit_data.rev.clone(),
            invite_code,
            deactivated: Some(deactivated),
        })
        .await
    {
        Ok(res) => {
            (access_jwt, refresh_jwt) = res;
        }
        Err(error) => {
            tracing::error!("Error creating account\n{error}");
            state.actor_store.destroy(&did, state.blobstore.clone()).await.map_err(|_| ApiError::RuntimeError)?;
            return Err(ApiError::RuntimeError);
        }
    }

    if !deactivated {
        let mut lock = state.sequencer.sequencer.write().await;
        if lock
            .sequence_identity_evt(did.clone(), Some(handle.clone()))
            .await
            .is_err()
        {
            tracing::error!("Sequence Identity Event failed");
        }
        if lock
            .sequence_account_evt(did.clone(), AccountStatus::Active)
            .await
            .is_err()
        {
            tracing::error!("Sequence Account Event failed");
        }
        if lock.sequence_commit(did.clone(), commit.clone()).await.is_err() {
            tracing::error!("Sequence Commit failed");
        }
        if lock
            .sequence_sync_evt(
                did.clone(),
                sync_evt_data_from_commit(commit.clone()).await.map_err(|_| ApiError::RuntimeError)?,
            )
            .await
            .is_err()
        {
            tracing::error!("Sequence sync event failed");
        }
    }
    state
        .account_manager
        .update_repo_root(did.clone(), commit.commit_data.cid, commit.commit_data.rev)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    let converted_did_doc = match did_doc {
        None => None,
        Some(did_doc) => match serde_json::to_value(did_doc) {
            Ok(res) => Some(res),
            Err(error) => {
                tracing::error!("Did Doc failed conversion\n{error}");
                return Err(ApiError::RuntimeError);
            }
        },
    };

    Ok(Json(CreateAccountOutput {
        access_jwt,
        refresh_jwt,
        handle,
        did,
        did_doc: converted_did_doc,
    }))
}

/// Validates Create Account Parameters and builds the PLC Operation if needed.
pub async fn validate_inputs_for_local_pds(
    state: &SharedState,
    input: CreateAccountInput,
    requester: Option<String>,
) -> Result<TransformedCreateAccountInput, ApiError> {
    let did: migration::types::did::Did;
    let plc_op;
    let deactivated: bool;
    let email;

    if input.plc_op.is_some() {
        return Err(ApiError::InvalidRequest(
            "Unsupported input: `plcOp`".to_string(),
        ));
    }

    let invite_code = if state.config.invites.required && input.invite_code.is_none() {
        return Err(ApiError::InvalidInviteCode);
    } else {
        input.invite_code.clone()
    };

    if input.email.is_none() {
        return Err(ApiError::InvalidEmail);
    }
    match input.email {
        None => return Err(ApiError::InvalidEmail),
        Some(ref input_email) => {
            let e_slice: &str = input_email.as_str();
            if !EmailAddress::is_valid(e_slice) {
                return Err(ApiError::InvalidEmail);
            } else {
                email = input_email.clone();
            }
        }
    }

    let handle = normalize_and_validate_handle(
        HandleValidationOpts {
            handle: input.handle.clone(),
            did: requester.clone(),
            allow_reserved: None,
        },
        &state.config.identity.service_handle_domains,
    )
    .await
    .map_err(|e| match e.kind {
        crate::handle::errors::ErrorKind::InvalidHandle => ApiError::InvalidHandle,
        crate::handle::errors::ErrorKind::HandleNotAvailable => ApiError::HandleNotAvailable,
        crate::handle::errors::ErrorKind::UnsupportedDomain => ApiError::UnsupportedDomain,
        crate::handle::errors::ErrorKind::InternalError => ApiError::RuntimeError,
    })?;
    if !super::validate_handle(&handle, &state.config.identity.service_handle_domains) {
        return Err(ApiError::InvalidHandle);
    }

    let handle_accnt = state
        .account_manager
        .get_account(&handle, None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let email_accnt = state
        .account_manager
        .get_account_by_email(&email, None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    if handle_accnt.is_some() {
        return Err(ApiError::HandleNotAvailable);
    } else if email_accnt.is_some() {
        return Err(ApiError::EmailNotAvailable);
    }

    let password = match input.password {
        None => return Err(ApiError::InvalidPassword),
        Some(ref pass) => pass.clone(),
    };

    match input.did {
        Some(input_did) => {
            if input_did == requester.unwrap_or("n/a".to_string()) {
                return Err(ApiError::AuthRequiredError(format!(
                    "Missing auth to create account with did: {input_did}"
                )));
            }
            did = input_did;
            plc_op = None;
            deactivated = true;
        }
        None => {
            let res = format_did_and_plc_op(input).await?;
            did = res.0;
            plc_op = Some(res.1);
            deactivated = false;
        }
    };

    Ok(TransformedCreateAccountInput {
        email,
        handle,
        did,
        invite_code,
        password,
        plc_op,
        deactivated,
    })
}

async fn format_did_and_plc_op(input: CreateAccountInput) -> Result<(String, Operation), ApiError> {
    let mut rotation_keys: Vec<String> = Vec::new();

    if let Some(recovery_key) = &input.recovery_key {
        rotation_keys.push(recovery_key.clone());
    }

    rotation_keys.push(encode_did_key(&PDS_PLC_ROTATION_KEYPAIR.public_key()));

    let create_op_input = CreateAtprotoOpInput {
        signing_key: encode_did_key(&PDS_REPO_SIGNING_KEYPAIR.public_key()),
        handle: input.handle,
        pds: format!(
            "https://{}",
            env::var("PDS_HOSTNAME").unwrap_or("localhost".to_owned())
        ),
        rotation_keys,
    };
    let response = match create_op(create_op_input, PDS_PLC_ROTATION_KEYPAIR.secret_key()).await {
        Ok(res) => res,
        Err(error) => {
            tracing::error!("{error}");
            return Err(ApiError::RuntimeError);
        }
    };

    Ok(response)
}

use crate::xrpc::com::atproto::server::safe_resolve_did_doc;
```

- [ ] **Step 3: Implement `activate_account.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/activate_account.rs`.

```rust
// pds/src/xrpc/com/atproto/server/activate_account.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::com::atproto::server::assert_valid_did_documents_for_service;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::State;
use rsky_syntax::handle::INVALID_HANDLE;

async fn inner_activate_account(
    auth: AccessFull,
    state: &SharedState,
) -> Result<(), ApiError> {
    let requester = auth.access.credentials.unwrap().did.unwrap();
    assert_valid_did_documents_for_service(requester.clone(), state.plc_client.as_ref())
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    let account = state
        .account_manager
        .get_account(
            &requester,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        state
            .account_manager
            .activate_account(&requester)
            .await
            .map_err(|_| ApiError::RuntimeError)?;

        let actor_store = state
            .actor_store
            .read(requester.clone(), state.blobstore.clone())
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        let sync_data = actor_store
            .get_sync_event_data()
            .await
            .map_err(|_| ApiError::RuntimeError)?;

        let status = state
            .account_manager
            .get_account_status(&requester)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        let mut lock = state.sequencer.sequencer.write().await;
        lock.sequence_account_evt(requester.clone(), status)
            .await
            .map_err(|_| ApiError::RuntimeError)?;

        let handle = account.handle.unwrap_or(INVALID_HANDLE.to_string());
        lock.sequence_identity_evt(requester.clone(), Some(handle))
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        lock.sequence_sync_evt(requester, sync_data)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        Ok(())
    } else {
        tracing::error!("User not found");
        Err(ApiError::RuntimeError)
    }
}

/// POST /xrpc/com.atproto.server.activateAccount
#[poem::handler]
pub async fn activate_account(
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_activate_account(auth, &state).await {
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}
```

- [ ] **Step 4: Implement `deactivate_account.rs`, `delete_account.rs`, `check_account_status.rs`**

```rust
// pds/src/xrpc/com/atproto/server/deactivate_account.rs
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::DeactivateAccountInput;

/// POST /xrpc/com.atproto.server.deactivateAccount
#[poem::handler]
pub async fn deactivate_account(
    body: Json<DeactivateAccountInput>,
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<()> {
    let did = auth.access.credentials.unwrap().did.unwrap();
    let DeactivateAccountInput { delete_after } = body.0;
    match state.account_manager.deactivate_account(&did, delete_after).await {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/delete_account.rs
use crate::account::helpers::account::{AccountStatus, AvailabilityFlags};
use crate::account::EmailTokenPurpose;
use crate::xrpc::auth_extractors::AdminToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::DeleteAccountInput;

async fn inner_delete_account(
    body: Json<DeleteAccountInput>,
    state: &SharedState,
) -> Result<(), ApiError> {
    let DeleteAccountInput {
        did,
        password,
        token,
    } = body.0;
    let account = state
        .account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    if account.is_some() {
        let valid_pass = state
            .account_manager
            .verify_account_password(&did, &password)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        if !valid_pass {
            return Err(ApiError::InvalidLogin);
        }
        state
            .account_manager
            .assert_valid_email_token(&did, EmailTokenPurpose::DeleteAccount, &token)
            .await
            .map_err(|_| ApiError::RuntimeError)?;

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
    } else {
        tracing::error!("account not found");
        Err(ApiError::RuntimeError)
    }
}

/// POST /xrpc/com.atproto.server.deleteAccount (admin token, per lexicon)
#[poem::handler]
pub async fn delete_account(
    body: Json<DeleteAccountInput>,
    _auth: AdminToken,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_delete_account(body, &state).await {
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/check_account_status.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::com::atproto::server::is_valid_did_doc_for_service;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use futures::try_join;
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::CheckAccountStatusOutput;

async fn inner_check_account_status(
    auth: AccessFull,
    state: &SharedState,
) -> Result<CheckAccountStatusOutput, ApiError> {
    let requester = auth.access.credentials.unwrap().did.unwrap();

    let mut actor_store = state
        .actor_store
        .read(requester.clone(), state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let repo_root = {
        let storage_guard = actor_store.storage.read().await;
        storage_guard.get_root_detailed().await.map_err(|_| ApiError::RuntimeError)?
    };
    let repo_blocks = {
        let storage_guard = actor_store.storage.read().await;
        storage_guard.count_blocks().await.map_err(|_| ApiError::RuntimeError)?
    };
    let (indexed_records, imported_blobs, expected_blobs) = try_join!(
        actor_store.record.record_count(),
        actor_store.blob.blob_count(),
        actor_store.blob.record_blob_count(),
    )
    .map_err(|_| ApiError::RuntimeError)?;

    let (activated, valid_did) = try_join!(
        state.account_manager.is_account_activated(&requester),
        is_valid_did_doc_for_service(requester.clone(), state.plc_client.as_ref()),
    )
    .map_err(|_| ApiError::RuntimeError)?;

    Ok(CheckAccountStatusOutput {
        activated,
        valid_did,
        repo_commit: repo_root.cid.to_string(),
        repo_rev: repo_root.rev,
        repo_blocks,
        indexed_records,
        private_state_values: 0,
        expected_blobs,
        imported_blobs,
    })
}

/// GET /xrpc/com.atproto.server.checkAccountStatus
#[poem::handler]
pub async fn check_account_status(
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<Json<CheckAccountStatusOutput>> {
    match inner_check_account_status(auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pds --test server_account_lifecycle_test`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add pds/src/xrpc/com/atproto/server/create_account.rs pds/src/xrpc/com/atproto/server/activate_account.rs pds/src/xrpc/com/atproto/server/deactivate_account.rs pds/src/xrpc/com/atproto/server/delete_account.rs pds/src/xrpc/com/atproto/server/check_account_status.rs pds/tests/server_account_lifecycle_test.rs
git commit -m "feat(server): account lifecycle handlers"
```
