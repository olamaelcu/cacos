//! XRPC route tree smoke tests.
//!
//! Pins the build_app wiring against the live SharedState produced by
//! [`crate::xrpc::test_utils::test_state`].

use cacos_pds_server::xrpc::build_app_with_state;
use cacos_pds_server::xrpc::test_utils;
use poem::test::TestClient;

#[tokio::test]
async fn health_returns_version() {
    let (state, _dirs) = test_utils::test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/_health").send().await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["version"], "0.0.0-test");
}

#[tokio::test]
async fn unknown_route_returns_xrpc_error_shape() {
    let (state, _dirs) = test_utils::test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/com.atproto.nope.missing").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InternalServerError");
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn cors_headers_present_on_health_live() {
    let (state, _dirs) = test_utils::test_state().await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/_health/live").send().await;
    resp.assert_status_is_ok();
    assert!(
        resp.0
            .content_type()
            .unwrap_or_default()
            .starts_with("text/plain"),
        "content-type must start with text/plain, got {:?}",
        resp.0.content_type()
    );
    let body = resp.0.into_body().into_string().await.unwrap();
    assert_eq!(body, "ok");
}
