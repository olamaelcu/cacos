# Task 28: Integration verification — full route tree, round-trip, metrics

**Files:**
- Modify: `pds/src/xrpc/mod.rs` (verify all module routes mount)
- Test: `pds/tests/integration_round_trip_test.rs`

- [ ] **Step 1: Write the failing integration test**

```rust
// pds/tests/integration_round_trip_test.rs
use pds::actor_store::test_helpers::seed_actor_repo;
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn full_round_trip() {
    let (state, _dirs) = test_state().await;

    // createAccount over HTTP
    let app = build_app(state.clone());
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.createAccount")
        .body_json(&json!({
            "handle": "roundtrip.test",
            "email": "roundtrip@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let created: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let did = created["did"].as_str().unwrap().to_string();
    let access = created["accessJwt"].as_str().unwrap().to_string();

    // the created account has an actor repo (createAccount seeds it), so
    // create a record
    let resp = cli
        .post("/xrpc/com.atproto.repo.createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "integration", "createdAt": "2026-01-01T00:00:00.000Z" }
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let record: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let uri = record["uri"].as_str().unwrap();
    let (_, path) = uri.split_once("at://").unwrap();
    let mut parts = path.split('/');
    let repo = parts.next().unwrap();
    let collection = parts.next().unwrap();
    let rkey = parts.next().unwrap();

    // read it back via repo.getRecord (local branch)
    let resp = cli
        .get(format!(
            "/xrpc/com.atproto.repo.getRecord?repo={repo}&collection={collection}&rkey={rkey}"
        ))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["value"]["text"], "integration");

    // sync.getLatestCommit sees the new commit
    let resp = cli
        .get(format!("/xrpc/com.atproto.sync.getLatestCommit?did={did}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    // session works
    let resp = cli
        .post("/xrpc/com.atproto.server.createSession")
        .body_json(&json!({ "identifier": "roundtrip.test", "password": "password123" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    // metrics route responds (Plan 02)
    let resp = cli.get("/metrics").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body = resp.0.into_body().into_string().await.unwrap();
    assert!(body.contains("cacos_http_requests_total"), "metrics body: {body}");
    assert!(body.contains("nsid"), "metrics body: {body}");
}

#[tokio::test]
async fn health_endpoints() {
    let (state, _dirs) = test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli.get("/xrpc/_health/live").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert_eq!(resp.0.into_body().into_string().await.unwrap(), "ok");
    let resp = cli.get("/xrpc/_health").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}
```

Run: `cargo test -p pds --test integration_round_trip_test`
Expected: FAIL — `createAccount` does not seed an actor repo in the test flow (the createAccount handler does seed it, so it should work; the metrics assertion may fail until the middleware records). Fix as needed.

- [ ] **Step 2: Wire the `/metrics` route through Plan 02's recorder**

Ensure `metrics::try_recorder()` returns the Plan 02 recorder. If the recorder is only installed at process start (`PrometheusBuilder::install_recorder`), tests must install it. Add to `test_utils::init_env` a recorder install guarded by a `Once`:

```rust
// append inside pds/src/xrpc/test_utils.rs
static INIT_RECORDER: Once = Once::new();

fn init_recorder() {
    INIT_RECORDER.call_once(|| {
        if metrics::try_recorder().is_none() {
            let _ = metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder();
        }
    });
}
```

and call `init_recorder()` at the top of `test_state()`.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test -p pds`
Expected: PASS — all unit + integration tests (Tasks 1–27 plus this task).

Run: `cargo clippy -p pds --all-targets -- -D warnings`
Expected: no clippy warnings (fix any raised).

- [ ] **Step 4: Verify the route tree end-to-end with a live server (manual smoke)**

Run:
```bash
PDS_HOSTNAME=localhost PDS_SERVICE_DID=did:web:localhost \
PDS_JWT_KEY_K256_PRIVATE_KEY_HEX=9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd \
PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX=71cfcf4882a6cff494c3d0affadd3858eb3a5838e7b5e15170e696a590a4fa01 \
PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX=e7763b0a2d1a4f8e9f9d2d6f3f9a5d0c6b2e4a8c1f7d3e9b5a0c2f4d6e8a0b1c \
PDS_DATA_DIRECTORY=/tmp/cacos-smoke PDS_ADMIN_PASSWORD=admin-password \
cargo run -p pds
```
Expected: server binds on port 2583. Then:
```bash
curl -s localhost:2583/xrpc/_health            # {"version":"0.3.0-beta.3"}
curl -s localhost:2583/xrpc/_health/live       # ok
curl -s localhost:2583/.well-known/atproto-did -H 'Host: alice.test'   # 404 "User not found"
curl -s localhost:2583/xrpc/com.atproto.server.describeServer            # JSON with service did
curl -s localhost:2583/metrics | grep cacos_http_requests_total         # counter present
```

- [ ] **Step 5: Commit**

```bash
git add pds/src/xrpc/mod.rs pds/src/xrpc/test_utils.rs pds/tests/integration_round_trip_test.rs
git commit -m "test(xrpc): integration round-trip and metrics verification"
```
