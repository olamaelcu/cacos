# Task 8: Email handlers — updateEmail, confirmEmail, requestEmailConfirmation, requestEmailUpdate, requestPasswordReset, resetPassword, requestAccountDelete

**Files:**
- Create: `pds/src/xrpc/com/atproto/server/update_email.rs`
- Create: `pds/src/xrpc/com/atproto/server/confirm_email.rs`
- Create: `pds/src/xrpc/com/atproto/server/request_email_confirmation.rs`
- Create: `pds/src/xrpc/com/atproto/server/request_email_update.rs`
- Create: `pds/src/xrpc/com/atproto/server/request_password_reset.rs`
- Create: `pds/src/xrpc/com/atproto/server/reset_password.rs`
- Create: `pds/src/xrpc/com/atproto/server/request_account_delete.rs`
- Test: `pds/tests/server_email_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/server_email_test.rs
use pds::account::EmailTokenPurpose;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn confirm_email_with_valid_token() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:a", "a.test").await;
    let token = state
        .account_manager
        .create_email_token("did:plc:a", EmailTokenPurpose::ConfirmEmail)
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.confirmEmail")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "email": "a.test@example.com", "token": token }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}

#[tokio::test]
async fn confirm_email_wrong_email_rejected() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:b", "b.test").await;
    let token = state
        .account_manager
        .create_email_token("did:plc:b", EmailTokenPurpose::ConfirmEmail)
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.confirmEmail")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "email": "other@example.com", "token": token }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidEmail");
}

#[tokio::test]
async fn request_password_reset_creates_token() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:c", "c.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.requestPasswordReset")
        .body_json(&json!({ "email": "c.test@example.com" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}

#[tokio::test]
async fn reset_password_with_token() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:d", "d.test").await;
    let token = state
        .account_manager
        .create_email_token("did:plc:d", EmailTokenPurpose::ResetPassword)
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.resetPassword")
        .body_json(&json!({ "token": token, "password": "newpass456" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert!(state
        .account_manager
        .verify_account_password("did:plc:d", &"newpass456".to_string())
        .await
        .unwrap());
}

#[tokio::test]
async fn request_account_delete_mints_token() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:e", "e.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.requestAccountDelete")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}

#[tokio::test]
async fn update_email_requires_token_for_confirmed_account() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:f", "f.test").await;
    // confirm the email so a token is required to change it
    let token = state
        .account_manager
        .create_email_token("did:plc:f", EmailTokenPurpose::ConfirmEmail)
        .await
        .unwrap();
    state
        .account_manager
        .confirm_email(pds::account::ConfirmEmailOpts {
            did: &"did:plc:f".to_string(),
            token: &token,
        })
        .await
        .unwrap();
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.updateEmail")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "email": "newf@example.com" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "RuntimeError"); // reference wraps handler errors as RuntimeError
}
```

Run: `cargo test -p pds --test server_email_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement the handlers**

Port each from the corresponding file under `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/`. The mailer calls are the Task 3 logging no-ops. `mailchecker::is_valid` (update_email) is replaced by a conservative `true` (no mailchecker dependency) — note this in the handler.

```rust
// pds/src/xrpc/com/atproto/server/confirm_email.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::{AccountManager, ConfirmEmailOpts};
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::ConfirmEmailInput;

async fn inner_confirm_email(
    body: ConfirmEmailInput,
    auth: AccessStandardIncludeChecks,
    account_manager: &AccountManager,
) -> Result<(), ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();

    let user = account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await
        .map_err(|e| {
            tracing::error!("Error: {e}");
            ApiError::RuntimeError
        })?;
    if let Some(user) = user {
        if let Some(user_email) = user.email {
            let ConfirmEmailInput { token, email } = body;
            if user_email != email.to_lowercase() {
                return Err(ApiError::InvalidEmail);
            }
            account_manager
                .confirm_email(ConfirmEmailOpts {
                    did: &did,
                    token: &token,
                })
                .await
                .map_err(|e| {
                    tracing::error!("Error: {e}");
                    ApiError::RuntimeError
                })?;
            Ok(())
        } else {
            Err(ApiError::InvalidRequest("Missing Email".to_string()))
        }
    } else {
        Err(ApiError::AccountNotFound)
    }
}

/// POST /xrpc/com.atproto.server.confirmEmail
#[poem::handler]
pub async fn confirm_email(
    body: Json<ConfirmEmailInput>,
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_confirm_email(body.0, auth, &state.account_manager).await {
        Ok(()) => Ok(()),
        Err(error) => Err(error),
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/update_email.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::{AccountManager, EmailTokenPurpose, UpdateEmailOpts};
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::UpdateEmailInput;

async fn inner_update_email(
    body: UpdateEmailInput,
    auth: AccessFull,
    account_manager: &AccountManager,
) -> Result<(), ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();
    let UpdateEmailInput { email, token } = body;
    // NOTE: rsky uses mailchecker::is_valid; cacos skips the third-party
    // mailchecker dependency and accepts any non-empty email.
    if email.is_empty() {
        return Err(ApiError::InvalidRequest(
            "This email address is not supported, please use a different email.".to_string(),
        ));
    }
    let account = account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        if account.email_confirmed_at.is_some() {
            if let Some(token) = token {
                account_manager
                    .assert_valid_email_token(&did, EmailTokenPurpose::UpdateEmail, &token)
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
            } else {
                return Err(ApiError::InvalidRequest(
                    "Confirmation token required".to_string(),
                ));
            }
        }
        account_manager
            .update_email(UpdateEmailOpts { did, email })
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("UserAlreadyExistsError") {
                    ApiError::InvalidRequest(
                        "This email address is already in use, please use a different email."
                            .to_string(),
                    )
                } else {
                    ApiError::RuntimeError
                }
            })
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.updateEmail
#[poem::handler]
pub async fn update_email(
    body: Json<UpdateEmailInput>,
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_update_email(body.0, auth, &state.account_manager).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(error)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/request_email_confirmation.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::EmailTokenPurpose;
use crate::mailer;
use crate::mailer::TokenParam;
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::State;

async fn inner_request_email_confirmation(
    auth: AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<(), ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();
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
    if let Some(account) = account {
        if let Some(email) = account.email {
            let token = state
                .account_manager
                .create_email_token(&did, EmailTokenPurpose::ConfirmEmail)
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            mailer::send_confirm_email(email, TokenParam { token })
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            Ok(())
        } else {
            Err(ApiError::InvalidRequest(
                "Account does not have an email address".to_string(),
            ))
        }
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.requestEmailConfirmation
#[poem::handler]
pub async fn request_email_confirmation(
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_request_email_confirmation(auth, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/request_email_update.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::EmailTokenPurpose;
use crate::mailer;
use crate::mailer::TokenParam;
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::RequestEmailUpdateOutput;

async fn inner_request_email_update(
    auth: AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<RequestEmailUpdateOutput, ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();
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
    if let Some(account) = account {
        if let Some(email) = account.email {
            let token_required = account.email_confirmed_at.is_some();
            if token_required {
                let token = state
                    .account_manager
                    .create_email_token(&did, EmailTokenPurpose::UpdateEmail)
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
                mailer::send_update_email(email, TokenParam { token })
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
            }

            Ok(RequestEmailUpdateOutput { token_required })
        } else {
            Err(ApiError::InvalidRequest(
                "Account does not have an email address".to_string(),
            ))
        }
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.requestEmailUpdate
#[poem::handler]
pub async fn request_email_update(
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<Json<RequestEmailUpdateOutput>> {
    match inner_request_email_update(auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/request_password_reset.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::EmailTokenPurpose;
use crate::mailer;
use crate::mailer::IdentifierAndTokenParams;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::RequestPasswordResetInput;

async fn inner_request_password_reset(
    body: RequestPasswordResetInput,
    state: &SharedState,
) -> Result<(), ApiError> {
    let RequestPasswordResetInput { email } = body;
    let email = email.to_lowercase();

    let account = state
        .account_manager
        .get_account_by_email(
            &email,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        if let Some(email) = account.email {
            let token = state
                .account_manager
                .create_email_token(&account.did, EmailTokenPurpose::ResetPassword)
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            mailer::send_reset_password(
                email.clone(),
                IdentifierAndTokenParams {
                    identifier: account.handle.unwrap_or(email),
                    token,
                },
            )
            .await
            .map_err(|_| ApiError::RuntimeError)?;
            Ok(())
        } else {
            Err(ApiError::InvalidRequest(
                "Account does not have an email address".to_string(),
            ))
        }
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.requestPasswordReset
#[poem::handler]
pub async fn request_password_reset(
    body: Json<RequestPasswordResetInput>,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_request_password_reset(body.0, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/reset_password.rs
use crate::account::{AccountManager, ResetPasswordOpts};
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::ResetPasswordInput;

/// POST /xrpc/com.atproto.server.resetPassword
#[poem::handler]
pub async fn reset_password(
    body: Json<ResetPasswordInput>,
    state: State<SharedState>,
) -> ApiResult<()> {
    let ResetPasswordInput { token, password } = body.0;
    match state
        .account_manager
        .reset_password(ResetPasswordOpts { token, password })
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
// pds/src/xrpc/com/atproto/server/request_account_delete.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::EmailTokenPurpose;
use crate::mailer;
use crate::mailer::TokenParam;
use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::State;

async fn inner_request_account_delete(
    auth: AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<(), ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();
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
    if let Some(account) = account {
        if let Some(email) = account.email {
            let token = state
                .account_manager
                .create_email_token(&did, EmailTokenPurpose::DeleteAccount)
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            mailer::send_account_delete(email, TokenParam { token })
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            Ok(())
        } else {
            Err(ApiError::InvalidRequest(
                "Account does not have an email address".to_string(),
            ))
        }
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.requestAccountDelete
#[poem::handler]
pub async fn request_account_delete(
    auth: AccessStandardIncludeChecks,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_request_account_delete(auth, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test server_email_test`
Expected: PASS (6 tests).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/server/update_email.rs pds/src/xrpc/com/atproto/server/confirm_email.rs pds/src/xrpc/com/atproto/server/request_email_confirmation.rs pds/src/xrpc/com/atproto/server/request_email_update.rs pds/src/xrpc/com/atproto/server/request_password_reset.rs pds/src/xrpc/com/atproto/server/reset_password.rs pds/src/xrpc/com/atproto/server/request_account_delete.rs pds/tests/server_email_test.rs
git commit -m "feat(server): email handlers (confirm/update/request/reset)"
```
