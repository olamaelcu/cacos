use poem::test::TestClient;

#[tokio::test(flavor = "multi_thread")]
async fn stub_app_serves_metrics_route() {
    // Use the metrics route directly so this test never depends on the
    // process-global OAuth env that `oauth_app_serves_jwks_and_metadata`
    // mutates (tests within a binary may run in parallel threads).
    cacos_pds_core::observability::metrics::init_metrics();
    let client = TestClient::new(cacos_pds_core::observability::http::metrics_route());
    let resp = client.get("/metrics").send().await;
    resp.assert_status_is_ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn oauth_app_serves_jwks_and_metadata() {
    // With the JWT key env set, the OAuth route set is mounted. Env is
    // process-global, so this test runs sequentially with the metrics test
    // (both are #[tokio::test(flavor = "multi_thread")] but the harness runs
    // tests one-at-a-time within a binary by default).
    let dir = camino_tempfile::Utf8TempDir::new().unwrap();
    unsafe {
        std::env::set_var(
            "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
            "4242424242424242424242424242424242424242424242424242424242424242",
        );
        std::env::set_var("PDS_SERVICE_DID", "did:web:pds.test");
        std::env::set_var("PDS_DB_PATH", dir.path().join("account.sqlite").as_str());
        std::env::set_var("PDS_PUBLIC_URL", "https://pds.test");
    }
    cacos_pds_core::observability::metrics::init_metrics();
    let client = TestClient::new(cacos_pds::xrpc::build_app().await);
    let resp = client.get("/oauth/jwks").send().await;
    resp.assert_status_is_ok();
    let resp = client
        .get("/.well-known/oauth-authorization-server")
        .send()
        .await;
    resp.assert_status_is_ok();
    // /metrics still served alongside the oauth routes.
    let resp = client.get("/metrics").send().await;
    resp.assert_status_is_ok();
}
