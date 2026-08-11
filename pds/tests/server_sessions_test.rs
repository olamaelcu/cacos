use cacos_pds_server::xrpc::build_app_with_state;
use cacos_pds_server::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn create_session_with_password() {
    let (state, _dirs) = test_state().await;
    let (_access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createSession")
        .body_json(&json!({ "identifier": "alice.test", "password": "password123" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["handle"], "alice.test");
    assert_eq!(body["did"], "did:plc:alice");
    assert!(body["accessJwt"].as_str().is_some());
    assert!(body["refreshJwt"].as_str().is_some());
}

#[tokio::test]
async fn create_session_bad_password_is_invalid_login() {
    let (state, _dirs) = test_state().await;
    let (_access, _refresh) = create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createSession")
        .body_json(&json!({ "identifier": "bob.test", "password": "wrong" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidLogin");
}

#[tokio::test]
async fn refresh_session_rotates_token() {
    let (state, _dirs) = test_state().await;
    let (_access, refresh) = create_test_account(&state, "did:plc:carol", "carol.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/refreshSession")
        .header("Authorization", format!("Bearer {refresh}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:carol");
    assert!(body["accessJwt"].as_str().is_some());
    assert_ne!(body["refreshJwt"].as_str().unwrap(), refresh);
}

#[tokio::test]
async fn get_session_returns_account() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:dan", "dan.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/getSession")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:dan");
    assert_eq!(body["handle"], "dan.test");
}

#[tokio::test]
async fn delete_session_revokes_refresh_token() {
    let (state, _dirs) = test_state().await;
    let (_access, refresh) = create_test_account(&state, "did:plc:erin", "erin.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/deleteSession")
        .header("Authorization", format!("Bearer {refresh}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    // the revoked refresh token can no longer refresh
    let resp = cli
        .post("/xrpc/refreshSession")
        .header("Authorization", format!("Bearer {refresh}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
}
