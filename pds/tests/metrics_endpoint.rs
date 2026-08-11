use metrics::counter;
use poem::test::TestClient;

#[tokio::test(flavor = "multi_thread")]
async fn metrics_endpoint_serves_prometheus_text() {
    cacos_pds_core::observability::metrics::init_metrics();
    counter!("cacos_http_requests_total").increment(1);

    let client = TestClient::new(cacos_pds_core::observability::http::metrics_route());
    let resp = client.get("/metrics").send().await;
    resp.assert_status_is_ok();

    let body = resp.0.into_body().into_string().await.expect("read body");
    assert!(
        body.contains("cacos_http_requests_total"),
        "missing metric in: {body}"
    );
    assert!(
        body.contains("cacos_http_requests_total 1"),
        "expected value 1 in: {body}"
    );
}
