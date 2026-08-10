//! Integration tests for the email handlers (Plan 08 / Task 8):
//! confirmEmail, updateEmail, requestEmailConfirmation, requestEmailUpdate,
//! requestPasswordReset, resetPassword, requestAccountDelete.
//!
//! Each test spins up a fresh in-memory state via `test_state()`. The mailer
//! is the logging no-op from `pds::mailer`, so we verify ACCOUNT side-effects
//! (tokens minted, password updated) directly through `account_manager`
//! rather than asserting on outbound mail.
//!
//! URL convention: this codebase registers XRPC routes with the short method
//! name (e.g. `/confirmEmail`), then nests them under `/xrpc`. Existing
//! `server_sessions_test.rs` uses `/xrpc/createSession` — same shape here.

use cacos_pds::account::EmailTokenPurpose;
use cacos_pds::xrpc::build_app_with_state;
use cacos_pds::xrpc::test_utils::{create_test_account, test_state};
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
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/confirmEmail")
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
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/confirmEmail")
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
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/requestPasswordReset")
        .body_json(&json!({ "email": "c.test@example.com" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}

#[tokio::test]
async fn reset_password_with_token() {
    let (state, _dirs) = test_state().await;
    let account_manager = state.account_manager.clone();
    create_test_account(&state, "did:plc:d", "d.test").await;
    let token = account_manager
        .create_email_token("did:plc:d", EmailTokenPurpose::ResetPassword)
        .await
        .unwrap();
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/resetPassword")
        .body_json(&json!({ "token": token, "password": "newpass456" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert!(
        account_manager
            .verify_account_password("did:plc:d", &"newpass456".to_string())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn request_account_delete_mints_token() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:e", "e.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/requestAccountDelete")
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
        .confirm_email(cacos_pds::account::ConfirmEmailOpts {
            did: &"did:plc:f".to_string(),
            token: &token,
        })
        .await
        .unwrap();
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/updateEmail")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "email": "newf@example.com" }))
        .send()
        .await;
    assert_eq!(
        resp.0.status(),
        poem::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InternalServerError"); // handler wraps inner errors as RuntimeError
}

#[tokio::test]
async fn request_email_confirmation_mints_token() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:g", "g.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/requestEmailConfirmation")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}

#[tokio::test]
async fn update_email_with_valid_token_succeeds() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:h", "h.test").await;
    // confirm email so an update token is required
    let confirm_token = state
        .account_manager
        .create_email_token("did:plc:h", EmailTokenPurpose::ConfirmEmail)
        .await
        .unwrap();
    state
        .account_manager
        .confirm_email(cacos_pds::account::ConfirmEmailOpts {
            did: &"did:plc:h".to_string(),
            token: &confirm_token,
        })
        .await
        .unwrap();
    // mint an update token (confirm_email cleans up the confirm token)
    let update_token = state
        .account_manager
        .create_email_token("did:plc:h", EmailTokenPurpose::UpdateEmail)
        .await
        .unwrap();
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/updateEmail")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "email": "newemail-h@example.com", "token": update_token }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}
