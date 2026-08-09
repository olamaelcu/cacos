use cacos_pds::xrpc::build_app_with_state;
use cacos_pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn create_list_revoke_app_password() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) =
        create_test_account(&state, "did:plc:a", "a.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createAppPassword")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "name": "My App" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let created: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(created["name"], "My App");
    assert_eq!(created["password"].as_str().unwrap().len(), 19);

    let resp = cli
        .get("/xrpc/listAppPasswords")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let listed: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(listed["passwords"].as_array().unwrap().len(), 1);
    assert_eq!(listed["passwords"][0]["name"], "My App");
    // ListAppPasswordsOutput entries must not include the password field.
    assert!(listed["passwords"][0].get("password").is_none());

    let resp = cli
        .post("/xrpc/revokeAppPassword")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "name": "My App" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    let resp = cli
        .get("/xrpc/listAppPasswords")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    let listed: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(listed["passwords"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn create_app_password_requires_auth() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createAppPassword")
        .body_json(&json!({ "name": "No Auth" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "AuthRequiredError");
}

#[tokio::test]
async fn duplicate_app_password_name_returns_runtime_error() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) =
        create_test_account(&state, "did:plc:dupe", "dupe.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createAppPassword")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "name": "Same Name" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    let resp = cli
        .post("/xrpc/createAppPassword")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "name": "Same Name" }))
        .send()
        .await;
    assert_eq!(
        resp.0.status(),
        poem::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InternalServerError");
}

#[tokio::test]
async fn list_app_passwords_is_empty_for_new_account() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) =
        create_test_account(&state, "did:plc:new", "new.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);

    let resp = cli
        .get("/xrpc/listAppPasswords")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(body["passwords"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn revoke_app_password_is_a_noop_for_unknown_name() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) =
        create_test_account(&state, "did:plc:noop", "noop.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/revokeAppPassword")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "name": "Does Not Exist" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}
