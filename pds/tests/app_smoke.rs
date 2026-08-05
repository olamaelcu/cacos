use poem::test::TestClient;

#[tokio::test(flavor = "multi_thread")]
async fn stub_app_serves_metrics_route() {
    cacos_pds::observability::metrics::init_metrics();
    let client = TestClient::new(cacos_pds::xrpc::build_app());
    let resp = client.get("/metrics").send().await;
    resp.assert_status_is_ok();
}
