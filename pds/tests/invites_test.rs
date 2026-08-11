use base64::Engine;
use cacos_pds_server::xrpc::build_app_with_state;
use cacos_pds_server::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

fn basic_auth_header() -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("admin:admin-password")
    )
}

#[tokio::test]
async fn create_invite_code_via_admin() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createInviteCode")
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
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/getAccountInviteCodes?includeUsed=false&createAvailable=false")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["codes"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn create_invite_codes_returns_batch_grouped_by_account() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createInviteCodes")
        .header("Authorization", basic_auth_header())
        .body_json(&json!({
            "codeCount": 2,
            "useCount": 1,
            "forAccounts": ["admin"],
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let codes = body["codes"].as_array().unwrap();
    assert_eq!(codes.len(), 1);
    let generated = codes[0]["codes"].as_array().unwrap();
    assert_eq!(generated.len(), 2);
    for code in generated {
        assert!(code.as_str().is_some());
    }
}
