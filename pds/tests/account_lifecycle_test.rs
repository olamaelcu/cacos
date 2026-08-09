//! Account lifecycle integration tests.
//!
//! Exercises `com.atproto.server.{createAccount, activateAccount,
//! deactivateAccount, deleteAccount, checkAccountStatus}` end-to-end
//! through the assembled poem app. Test isolation: each test gets its
//! own temp-directory-backed [`SharedState`] via [`test_state`].

use cacos_pds::xrpc::build_app_with_state;
use cacos_pds::xrpc::test_utils::test_state;
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn create_account_success() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createAccount")
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
    assert!(body["refreshJwt"].as_str().is_some());
}

#[tokio::test]
async fn create_account_requires_email() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({ "handle": "noemail.test", "password": "password123" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidEmail");
}

#[tokio::test]
async fn create_account_rejects_invalid_handle() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({
            "handle": "!!! invalid !!!",
            "email": "x@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidHandle");
}

#[tokio::test]
async fn create_account_duplicate_handle_rejected() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({
            "handle": "dup.test",
            "email": "dup1@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let resp = cli
        .post("/xrpc/createAccount")
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
async fn create_account_requires_password() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({
            "handle": "nopw.test",
            "email": "nopw@example.com",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidPassword");
}

#[tokio::test]
async fn deactivate_account_returns_ok() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({
            "handle": "deact.test",
            "email": "deact@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let access = body["accessJwt"].as_str().unwrap();

    let resp = cli
        .post("/xrpc/deactivateAccount")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "deleteAfter": null }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}

#[tokio::test]
async fn deactivate_account_requires_auth() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/deactivateAccount")
        .body_json(&json!({ "deleteAfter": null }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "AuthRequiredError");
}

#[tokio::test]
async fn check_account_status_returns_activated_true_after_create() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({
            "handle": "statuser.test",
            "email": "statuser@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let access = body["accessJwt"].as_str().unwrap();

    let resp = cli
        .get("/xrpc/checkAccountStatus")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["activated"], true);
    assert!(body["repoCommit"].is_string());
    assert!(body["repoRev"].is_string());
}
