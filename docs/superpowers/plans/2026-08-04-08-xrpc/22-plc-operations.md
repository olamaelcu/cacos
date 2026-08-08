# Task 22: submitPlcOperation, signPlcOperation, requestPlcOperationSignature

**Files:**
- Create: `pds/src/xrpc/com/atproto/identity/submit_plc_operation.rs`
- Create: `pds/src/xrpc/com/atproto/identity/sign_plc_operation.rs`
- Create: `pds/src/xrpc/com/atproto/identity/request_plc_operation_signature.rs`
- Test: `pds/tests/identity_plc_ops_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/identity_plc_ops_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn request_plc_operation_signature_mints_email_token() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.identity.requestPlcOperationSignature")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}

#[tokio::test]
async fn sign_plc_operation_requires_token() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.identity.signPlcOperation")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "token": "", "alsoKnownAs": ["at://bob.test"] }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidRequest");
}

#[tokio::test]
async fn submit_plc_operation_rejects_missing_rotation_key() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:carol", "carol.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.identity.submitPlcOperation")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "operation": {
                "type": "plc_operation",
                "rotationKeys": ["did:key:zWrong"],
                "verificationMethods": {},
                "alsoKnownAs": [],
                "services": {},
                "prev": null,
                "sig": null
            }
        }))
        .send()
        .await;
    // rotation keys do not include the server's key → InvalidRequest
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidRequest");
}
```

Run: `cargo test -p pds --test identity_plc_ops_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `request_plc_operation_signature.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/identity/request_plc_operation_signature.rs` (mailer is the Task 3 logging no-op).

```rust
// pds/src/xrpc/com/atproto/identity/request_plc_operation_signature.rs
use crate::account::helpers::account::{ActorAccount, AvailabilityFlags};
use crate::account::EmailTokenPurpose;
use crate::mailer;
use crate::mailer::TokenParam;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::State;

async fn get_requester_did(auth: &AccessFull) -> Result<String, ApiError> {
    match &auth.access.credentials {
        None => {
            tracing::error!("Failed to find access credentials");
            Err(ApiError::RuntimeError)
        }
        Some(res) => match &res.did {
            None => {
                tracing::error!("Failed to find did");
                Err(ApiError::RuntimeError)
            }
            Some(did) => Ok(did.clone()),
        },
    }
}

async fn get_account(
    requester_did: &str,
    state: &SharedState,
) -> Result<ActorAccount, ApiError> {
    let availability_flags = AvailabilityFlags {
        include_taken_down: Some(true),
        include_deactivated: Some(true),
    };
    match state
        .account_manager
        .get_account(requester_did, Some(availability_flags))
        .await
    {
        Ok(account) => match account {
            None => {
                tracing::error!("Account not found despite valid credentials");
                Err(ApiError::RuntimeError)
            }
            Some(account) => Ok(account),
        },
        Err(error) => {
            tracing::error!("Error getting account\n{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

async fn create_email_token(requester: &str, state: &SharedState) -> Result<String, ApiError> {
    match state
        .account_manager
        .create_email_token(requester, EmailTokenPurpose::PlcOperation)
        .await
    {
        Ok(res) => Ok(res),
        Err(error) => {
            tracing::error!("Failed to create plc operation token\n{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

async fn do_plc_operation(account: &ActorAccount, token: String) -> Result<(), ApiError> {
    match &account.email {
        None => {
            tracing::error!("Failed to find email for account");
            Err(ApiError::RuntimeError)
        }
        Some(email) => match mailer::send_plc_operation(email.clone(), TokenParam { token }).await {
            Ok(_) => Ok(()),
            Err(error) => {
                tracing::error!("Failed to send PLC Operation Token Email\n{error}");
                Err(ApiError::RuntimeError)
            }
        },
    }
}

/// POST /xrpc/com.atproto.identity.requestPlcOperationSignature
#[poem::handler]
pub async fn request_plc_operation_signature(
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<()> {
    let requester = get_requester_did(&auth).await?;
    let account = get_account(requester.as_str(), &state).await?;
    let token = create_email_token(requester.as_str(), &state).await?;
    do_plc_operation(&account, token).await?;

    Ok(())
}
```

- [ ] **Step 3: Implement `sign_plc_operation.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/identity/sign_plc_operation.rs`.

```rust
// pds/src/xrpc/com/atproto/identity/sign_plc_operation.rs
use crate::account::EmailTokenPurpose;
use crate::plc::operations::create_update_op;
use crate::plc::types::{CompatibleOp, CompatibleOpOrTombstone, Operation, Service};
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::identity::SignPlcOperationRequest;
use std::collections::BTreeMap;

/// POST /xrpc/com.atproto.identity.signPlcOperation
#[poem::handler]
pub async fn sign_plc_operation(
    body: Json<SignPlcOperationRequest>,
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<Json<Operation>> {
    let did = auth.access.credentials.unwrap().did.unwrap();
    let request = body.0;
    let token = request.token.clone();

    if request.token.is_empty() {
        return Err(ApiError::InvalidRequest(
            "email confirmation token required to sign PLC operations".to_string(),
        ));
    }
    state
        .account_manager
        .assert_valid_email_token_and_cleanup(&did, EmailTokenPurpose::PlcOperation, &token)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    let last_op: CompatibleOp = match state.plc_client.get_last_op(&did).await {
        Ok(res) => match res {
            CompatibleOpOrTombstone::CreateOpV1(op) => CompatibleOp::CreateOpV1(op),
            CompatibleOpOrTombstone::Operation(op) => CompatibleOp::Operation(op),
            CompatibleOpOrTombstone::Tombstone(_) => {
                return Err(ApiError::InvalidRequest("Did is tombstoned".to_string()))
            }
        },
        Err(error) => {
            tracing::error!("Error getting last PLC operation\n{error}");
            return Err(ApiError::RuntimeError);
        }
    };

    let also_known_as = match request.also_known_as {
        None => match last_op {
            CompatibleOp::CreateOpV1(_) => None,
            CompatibleOp::Operation(ref op) => Some(op.also_known_as.clone()),
        },
        Some(res) => Some(res),
    };
    let services = match request.services {
        None => match last_op {
            CompatibleOp::CreateOpV1(_) => None,
            CompatibleOp::Operation(ref op) => Some(op.services.clone()),
        },
        Some(res) => match serde_json::from_value::<BTreeMap<String, Service>>(res) {
            Ok(services) if !services.is_empty() => Some(services),
            _ => match last_op {
                CompatibleOp::CreateOpV1(_) => None,
                CompatibleOp::Operation(ref op) => Some(op.services.clone()),
            },
        },
    };
    let verification_methods = match request.verification_methods {
        None => match last_op {
            CompatibleOp::CreateOpV1(_) => None,
            CompatibleOp::Operation(ref op) => Some(op.verification_methods.clone()),
        },
        Some(res) => Some(res),
    };
    let rotation_keys = match request.rotation_keys {
        None => match last_op {
            CompatibleOp::CreateOpV1(_) => None,
            CompatibleOp::Operation(ref op) => Some(op.rotation_keys.clone()),
        },
        Some(res) => Some(res),
    };
    let operation = match create_update_op(
        last_op,
        &PDS_PLC_ROTATION_KEYPAIR.secret_key(),
        |normalized: Operation| -> Operation {
            let mut updated = normalized.clone();
            if let Some(also_known_as) = &also_known_as {
                updated.also_known_as = also_known_as.clone();
            }
            if let Some(services) = &services {
                updated.services = services.clone();
            }
            if let Some(verification_methods) = &verification_methods {
                updated.verification_methods = verification_methods.clone();
            }
            if let Some(rotation_keys) = &rotation_keys {
                updated.rotation_keys = rotation_keys.clone();
            }
            updated
        },
    )
    .await
    {
        Ok(res) => res,
        Err(error) => {
            tracing::error!("Error creating signed operation\n{error}");
            return Err(ApiError::RuntimeError);
        }
    };

    Ok(Json(operation))
}
```

- [ ] **Step 4: Implement `submit_plc_operation.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/identity/submit_plc_operation.rs`.

```rust
// pds/src/xrpc/com/atproto/identity/submit_plc_operation.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::context::PDS_REPO_SIGNING_KEYPAIR;
use crate::plc::types::{OpOrTombstone, Operation};
use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_crypto::utils::encode_did_key;
use rsky_lexicon::com::atproto::identity::SubmitPlcOperationRequest;

fn get_requester_did(auth: &AccessStandard) -> Result<String, ApiError> {
    match &auth.access.credentials {
        None => {
            tracing::error!("Failed to find access credentials");
            Err(ApiError::RuntimeError)
        }
        Some(res) => match &res.did {
            None => {
                tracing::error!("Failed to find did");
                Err(ApiError::RuntimeError)
            }
            Some(did) => Ok(did.clone()),
        },
    }
}

async fn validate_plc_request(
    did: &str,
    op: &Operation,
    public_endpoint: &str,
    state: &SharedState,
) -> Result<(), ApiError> {
    let public_rotation_key = encode_did_key(&PDS_PLC_ROTATION_KEYPAIR.public_key());
    if !op.rotation_keys.contains(&public_rotation_key) {
        return Err(ApiError::InvalidRequest(
            "Rotation keys do not include server's rotation key".to_string(),
        ));
    }

    let public_signing_key = encode_did_key(&PDS_REPO_SIGNING_KEYPAIR.public_key());
    match op.verification_methods.get("atproto") {
        None => {
            return Err(ApiError::InvalidRequest(
                "Incorrect signing key".to_string(),
            ))
        }
        Some(res) => {
            if res.clone() != public_signing_key {
                return Err(ApiError::InvalidRequest(
                    "Incorrect signing key".to_string(),
                ));
            }
        }
    }

    let services = op.services.get("atproto_pds");
    match services {
        None => return Err(ApiError::InvalidRequest("Missing atproto_pds".to_string())),
        Some(res) => {
            if res.r#type != "AtprotoPersonalDataServer" {
                return Err(ApiError::InvalidRequest(
                    "Incorrect type on atproto_pds service".to_string(),
                ));
            }
            if res.endpoint != *public_endpoint {
                return Err(ApiError::InvalidRequest(
                    "Incorrect endpoint on atproto_pds service".to_string(),
                ));
            }
        }
    }

    let account = state
        .account_manager
        .get_account(
            did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let account = match account {
        None => {
            tracing::error!("Unable to find account with valid token");
            return Err(ApiError::RuntimeError);
        }
        Some(actor_account) => actor_account,
    };
    if let Some(handle) = account.handle {
        let op_handle = match op.also_known_as.first() {
            None => {
                return Err(ApiError::InvalidRequest(
                    "No handle provided in operation".to_string(),
                ))
            }
            Some(handle) => handle.clone(),
        };

        if op_handle != format!("at://{handle}") {
            return Err(ApiError::InvalidRequest(
                "Incorrect handle in alsoKnownAs".to_string(),
            ));
        }
    }

    Ok(())
}

async fn do_plc_operation(plc_url: &str, did: &str, op: Operation, state: &SharedState) -> Result<(), ApiError> {
    match state
        .plc_client
        .send_operation(&did.to_string(), &OpOrTombstone::Operation(op))
        .await
    {
        Ok(_res) => {
            tracing::info!("Successfully sent PLC Update Operation");
            Ok(())
        }
        Err(error) => {
            tracing::error!("Failed to update did:plc\n{error}");
            Err(ApiError::RuntimeError)
        }
    }
}

fn validate_operation_body(request: SubmitPlcOperationRequest) -> Result<Operation, ApiError> {
    match serde_json::from_value::<Operation>(request.operation) {
        Ok(op) => Ok(op),
        Err(error) => {
            tracing::error!("Error parsing operation body\n{error}");
            Err(ApiError::InvalidRequest("Invalid operation".to_string()))
        }
    }
}

/// POST /xrpc/com.atproto.identity.submitPlcOperation
#[poem::handler]
pub async fn submit_plc_operation(
    body: Json<SubmitPlcOperationRequest>,
    auth: AccessStandard,
    state: State<SharedState>,
) -> ApiResult<()> {
    let did = get_requester_did(&auth)?;

    let op = validate_operation_body(body.0)?;

    validate_plc_request(
        did.as_str(),
        &op,
        state.config.service.public_url.as_str(),
        &state,
    )
    .await?;

    do_plc_operation(
        state.config.identity.plc_url.as_str(),
        did.as_str(),
        op,
        &state,
    )
    .await?;

    let mut seq_lock = state.sequencer.sequencer.write().await;
    seq_lock
        .sequence_identity_evt(did.clone(), None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    let id_lock = state.id_resolver.write().await;
    if let Err(error) = id_lock.did.ensure_resolve(&did, None).await {
        tracing::error!("Failed to fresh did after plc update\n{error}")
    };

    Ok(())
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pds --test identity_plc_ops_test`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add pds/src/xrpc/com/atproto/identity/submit_plc_operation.rs pds/src/xrpc/com/atproto/identity/sign_plc_operation.rs pds/src/xrpc/com/atproto/identity/request_plc_operation_signature.rs pds/tests/identity_plc_ops_test.rs
git commit -m "feat(identity): submitPlcOperation, signPlcOperation, requestPlcOperationSignature"
```
