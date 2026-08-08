use cacos_pds::xrpc::build_app_with_state;
use cacos_pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;

#[tokio::test]
async fn well_known_returns_did_for_local_account() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .get("/.well-known/atproto-did")
        .header("Host", "alice.test")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert_eq!(
        resp.0.into_body().into_string().await.unwrap(),
        "did:plc:alice"
    );
}

#[tokio::test]
async fn well_known_404_for_unknown_handle() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .get("/.well-known/atproto-did")
        .header("Host", "nobody.test")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
    assert_eq!(
        resp.0.into_body().into_string().await.unwrap(),
        "User not found"
    );
}

#[tokio::test]
async fn well_known_404_for_unsupported_host() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .get("/.well-known/atproto-did")
        .header("Host", "evil.example.com")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
}