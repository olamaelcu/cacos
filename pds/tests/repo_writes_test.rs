//! `com.atproto.repo.*` write handlers integration tests.
//!
//! Verifies createRecord / putRecord / applyWrites / uploadBlob /
//! deleteRecord / describeRepo end-to-end through the assembled
//! poem app, and asserts that the `repo_load`, `repo_write`,
//! `seq_write`, and `blob_put` stage timing histograms are populated.

use cacos_pds::auth::auth_verifier::_reset_auth_dependencies_for_tests;
use cacos_pds::auth::auth_verifier::register_auth_dependencies;
use cacos_pds::context::PDS_REPO_SIGNING_KEYPAIR;
use cacos_pds::observability::metrics as obs_metrics;
use cacos_pds::observability::metrics::TIMING_STAGE_SECONDS;
use cacos_pds::xrpc::SharedState;
use cacos_pds::xrpc::build_app_with_state;
use cacos_pds::xrpc::test_utils::{create_test_account, test_state};
use poem::http::StatusCode;
use poem::test::TestClient;
use serde_json::{Value, json};
use std::sync::OnceLock;

/// Process-wide shared `SharedState`. Built once per test process so
/// that parallel tests all reference the same account database
/// (the global `auth_verifier::ACCOUNT_MANAGER` is process-global and
/// would otherwise point to whichever test initialised it first).
fn shared_state() -> &'static (SharedState, Vec<tempfile::TempDir>) {
    static STATE: OnceLock<(SharedState, Vec<tempfile::TempDir>)> = OnceLock::new();
    STATE.get_or_init(|| {
        obs_metrics::init_metrics();
        let (state, dirs) = futures::executor::block_on(test_state());
        (state, dirs)
    })
}

async fn setup_env() -> &'static SharedState {
    _reset_auth_dependencies_for_tests();
    let (state, _dirs) = shared_state();
    register_auth_dependencies(std::sync::Arc::new(state.account_manager.clone()), None);
    state
}

async fn read(resp: poem::test::TestResponse) -> (StatusCode, Value) {
    let status = resp.0.status();
    let body_text = resp.0.into_body().into_string().await.unwrap_or_default();
    let body: Value = if body_text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&body_text).unwrap_or(Value::Null)
    };
    (status, body)
}

/// Create an account and initialize its actor store + empty repo
/// directly through the in-process `SharedState`. We avoid the HTTP
/// `createAccount` endpoint because the global auth dependency is a
/// `OnceLock` and only honors the first test's registration.
async fn setup_repo_account(state: &SharedState, did: &str, handle: &str) -> String {
    state
        .actor_store
        .create(did, &PDS_REPO_SIGNING_KEYPAIR)
        .await
        .expect("actor_store.create");
    let transactor = state
        .actor_store
        .transact(did.to_owned(), state.blobstore.clone())
        .await
        .expect("actor_store.transact");
    let commit = transactor
        .create_repo(Vec::new())
        .await
        .expect("transactor.create_repo");
    state
        .account_manager
        .update_repo_root(
            did.to_owned(),
            commit.commit_data.cid,
            commit.commit_data.rev.clone(),
        )
        .await
        .expect("update_repo_root");
    let (access, _refresh) = create_test_account(state, did, handle).await;
    access
}

fn assert_stage_observed(stage: &'static str) {
    let snapshot = obs_metrics::render();
    let needle_count = format!("cacos_timing_seconds_count{{stage=\"{stage}\"}}");
    let needle_sum = format!("cacos_timing_seconds_sum{{stage=\"{stage}\"}}");
    assert!(
        snapshot.contains(&needle_count) || snapshot.contains(&needle_sum),
        "expected stage histogram sample for stage={stage} in: {snapshot}"
    );
}

fn assert_label_set_is_bounded(dodgy_substrings: &[&str]) {
    let snapshot = obs_metrics::render();
    for line in snapshot.lines() {
        if line.starts_with("# HELP cacos_timing_seconds")
            || line.starts_with("# TYPE cacos_timing_seconds")
            || line.is_empty()
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("cacos_timing_seconds") {
            for sub in dodgy_substrings {
                assert!(!rest.contains(sub), "stage label leaks {sub}: {line}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_record_writes_and_emits_seq_event() {
    // no lock needed: shared state + reset/register global per test
    // init_metrics is invoked once by shared_state()
    let state = setup_env().await;
    let did = "did:plc:writer".to_string();
    let access = setup_repo_account(state, &did, "writer.test").await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "collection": "app.bsky.feed.post",
            "record": {
                "$type": "app.bsky.feed.post",
                "text": "hello world",
                "createdAt": "2026-01-01T00:00:00.000Z",
            },
        }))
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "createRecord failed: {body}");
    let uri = body["uri"].as_str().unwrap();
    assert!(uri.starts_with("at://"), "uri should be at:// uri: {uri}");

    assert_stage_observed("repo_load");
    assert_stage_observed("repo_write");
    assert_stage_observed("seq_write");
    assert_label_set_is_bounded(&["did:plc:", "at://", "bafy", "did:web:"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_record_requires_auth() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createRecord")
        .body_json(&json!({
            "repo": "did:plc:ghost",
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "x", "createdAt": "2026-01-01T00:00:00.000Z" },
        }))
        .send()
        .await;
    let (status, _) = read(resp).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn put_record_updates_existing_post() {
    // no lock needed: shared state + reset/register global per test
    let state = setup_env().await;
    let did = "did:plc:upd".to_string();
    let access = setup_repo_account(state, &did, "upd.test").await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "collection": "app.bsky.actor.profile",
            "record": { "$type": "app.bsky.actor.profile", "displayName": "v1" },
        }))
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "createRecord failed: {body}");
    let uri = body["uri"].as_str().unwrap().to_string();

    let resp = cli
        .post("/xrpc/putRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "collection": "app.bsky.actor.profile",
            "rkey": uri.rsplit('/').next().unwrap(),
            "record": { "$type": "app.bsky.actor.profile", "displayName": "v2" },
        }))
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "putRecord failed: {body}");
    assert!(
        body["cid"].as_str().is_some(),
        "putRecord should return a cid"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_writes_dispatches_multiple_actions() {
    // no lock needed: shared state + reset/register global per test
    let state = setup_env().await;
    let did = "did:plc:batch".to_string();
    let access = setup_repo_account(state, &did, "batch.test").await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/applyWrites")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "writes": [
                {
                    "$type": "com.atproto.repo.applyWrites#create",
                    "collection": "app.bsky.feed.post",
                    "value": { "$type": "app.bsky.feed.post", "text": "a", "createdAt": "2026-01-01T00:00:00.000Z" },
                },
                {
                    "$type": "com.atproto.repo.applyWrites#create",
                    "collection": "app.bsky.feed.post",
                    "value": { "$type": "app.bsky.feed.post", "text": "b", "createdAt": "2026-01-01T00:00:00.000Z" },
                },
            ],
        }))
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "applyWrites failed: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_blob_stores_and_returns_ref() {
    // no lock needed: shared state + reset/register global per test
    // init_metrics is invoked once by shared_state()
    let state = setup_env().await;
    let did = "did:plc:blob".to_string();
    let access = setup_repo_account(state, &did, "blob.test").await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/uploadBlob")
        .header("Authorization", format!("Bearer {access}"))
        .body("hello".as_bytes().to_vec())
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "uploadBlob failed: {body}");
    assert_eq!(body["blob"]["mimeType"], "application/octet-stream");
    assert_eq!(body["blob"]["size"], 5);
    let cid = body["blob"]["ref"]["$link"].as_str().expect("ref link");
    assert!(
        cid.starts_with("bafk"),
        "expected CID-shaped blob ref, got: {cid}"
    );
    assert!(cid.len() >= 30, "expected full CID, got short value: {cid}");

    assert_stage_observed("blob_put");
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_blob_metrics_render_works() {
    // init_metrics is invoked once by shared_state(). Touch the shared
    // state here so the recorder is set even if this test happens to run
    // first in the parallel test shuffle.
    let _ = setup_env().await;
    metrics::counter!("cacos_test_increment").increment(1);
    let snapshot = obs_metrics::render();
    assert!(
        snapshot.contains("cacos_test_increment"),
        "expected counter increment in: {snapshot}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_record_removes_post() {
    // no lock needed: shared state + reset/register global per test
    let state = setup_env().await;
    let did = "did:plc:del".to_string();
    let access = setup_repo_account(state, &did, "del.test").await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "temporary", "createdAt": "2026-01-01T00:00:00.000Z" },
        }))
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "createRecord failed: {body}");
    let uri = body["uri"].as_str().unwrap().to_string();
    let rkey = uri.rsplit('/').next().unwrap().to_string();

    let resp = cli
        .post("/xrpc/deleteRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "collection": "app.bsky.feed.post",
            "rkey": rkey,
        }))
        .send()
        .await;
    let (status, _body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "deleteRecord failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_repo_returns_collections() {
    // no lock needed: shared state + reset/register global per test
    let state = setup_env().await;
    let did = "did:plc:desc".to_string();
    let access = setup_repo_account(state, &did, "desc.test").await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createRecord")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({
            "repo": did,
            "collection": "app.bsky.feed.post",
            "record": { "$type": "app.bsky.feed.post", "text": "hello", "createdAt": "2026-01-01T00:00:00.000Z" },
        }))
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "createRecord failed: {body}");

    let resp = cli
        .get("/xrpc/describeRepo")
        .query("repo", &did)
        .send()
        .await;
    let (status, body) = read(resp).await;
    assert_eq!(status, StatusCode::OK, "describeRepo failed: {body}");
    let collections = body["collections"].as_array().unwrap();
    assert!(
        collections
            .iter()
            .any(|c| c.as_str() == Some("app.bsky.feed.post")),
        "describeRepo should list the post collection, got: {body}"
    );
}

#[tokio::test]
async fn timing_stage_histogram_constant_is_well_formed() {
    assert_eq!(TIMING_STAGE_SECONDS, "cacos_timing_seconds");
}
