//! Per-IP rate limiting, per-account login lockout, and password-reset
//! enumeration-defense integration tests.

use cacos_pds_server::xrpc::build_app_with_state;
use cacos_pds_server::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

const TEST_IP: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));

fn set_test_ratelimits() {
    // Generous budget so the lockout tests can exercise 5 failed logins
    // without tripping the per-IP cap. The rate_limit_blocks_after_threshold
    // test bumps PDS_RATELIMIT_CREATE_SESSION_PER_MINUTE down to a tight
    // value locally to verify the 429 path.
    // SAFETY: tests run sequentially within a process.
    unsafe {
        std::env::set_var("PDS_RATELIMIT_CREATE_SESSION_PER_MINUTE", "100");
        std::env::set_var("PDS_RATELIMIT_CREATE_ACCOUNT_PER_MINUTE", "100");
        std::env::set_var("PDS_RATELIMIT_PASSWORD_RESET_PER_MINUTE", "100");
        std::env::set_var("PDS_RATELIMIT_EMAIL_OPS_PER_MINUTE", "100");
    }
}

fn set_tight_session_ratelimit() {
    unsafe {
        std::env::set_var("PDS_RATELIMIT_CREATE_SESSION_PER_MINUTE", "2");
    }
}

#[tokio::test]
async fn rate_limit_blocks_after_threshold() {
    set_test_ratelimits();
    set_tight_session_ratelimit();
    let (state, _dirs) = test_state().await;
    let (_access, _refresh) =
        create_test_account(&state, "did:plc:ratelimit", "ratelimit.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    // 2 requests within the limit must succeed (login will succeed
    // because we use the real password).
    for i in 0..2 {
        let resp = cli
            .post("/xrpc/createSession")
            .body_json(&json!({
                "identifier": "ratelimit.test",
                "password": "password123",
            }))
            .send()
            .await;
        assert_eq!(
            resp.0.status(),
            poem::http::StatusCode::OK,
            "createSession #{i} within rate-limit budget should succeed"
        );
    }
    // 3rd request hits the per-IP token-bucket cap and is rejected with 429.
    let resp = cli
        .post("/xrpc/createSession")
        .body_json(&json!({
            "identifier": "ratelimit.test",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(
        resp.0.status(),
        poem::http::StatusCode::TOO_MANY_REQUESTS,
        "createSession over the per-IP limit must return 429"
    );
    // Body should be the standard RateLimitExceeded JSON shape.
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "RateLimitExceeded");

    let _ = TEST_IP;
}

#[tokio::test]
async fn login_lockout_triggers_after_5_failures() {
    set_test_ratelimits();
    let (state, _dirs) = test_state().await;
    let (_access, _refresh) = create_test_account(&state, "did:plc:lockout", "lockout.test").await;
    // Rebuild the app per attempt so each gets a fresh per-IP budget.
    for attempt in 1..=5 {
        let app = build_app_with_state(state.clone()).await;
        let cli = TestClient::new(app);
        let resp = cli
            .post("/xrpc/createSession")
            .body_json(&json!({
                "identifier": "lockout.test",
                "password": "wrong",
            }))
            .send()
            .await;
        assert_eq!(
            resp.0.status(),
            poem::http::StatusCode::BAD_REQUEST,
            "wrong password attempt #{attempt} should be InvalidLogin (400)"
        );
    }
    // Sixth attempt with the correct password must be rejected: the
    // account is locked after 5 failed logins.
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/createSession")
        .body_json(&json!({
            "identifier": "lockout.test",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(
        resp.0.status(),
        poem::http::StatusCode::TOO_MANY_REQUESTS,
        "correct password must be rejected after 5 failed attempts"
    );
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "RateLimitExceeded");
}

#[tokio::test]
async fn login_lockout_resets_after_successful_login() {
    set_test_ratelimits();
    let (state, _dirs) = test_state().await;
    let db = state.account_manager.db.clone();
    let (_access, _refresh) =
        create_test_account(&state, "did:plc:resetlock", "resetlock.test").await;
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    // Four bad attempts (one short of the 5-failure threshold).
    for _ in 0..4 {
        let resp = cli
            .post("/xrpc/createSession")
            .body_json(&json!({
                "identifier": "resetlock.test",
                "password": "wrong",
            }))
            .send()
            .await;
        assert_eq!(
            resp.0.status(),
            poem::http::StatusCode::BAD_REQUEST,
            "wrong password should be InvalidLogin (400)"
        );
    }
    // One successful attempt clears the failed-login counter.
    let resp = cli
        .post("/xrpc/createSession")
        .body_json(&json!({
            "identifier": "resetlock.test",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(
        resp.0.status(),
        poem::http::StatusCode::OK,
        "successful login should clear the failed-login counter"
    );
    // The account must not be locked: `locked_until` is NULL.
    let (count, locked_until) =
        cacos_pds_account::account::helpers::account::get_account_lockout_state(
            "did:plc:resetlock",
            &db,
        )
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "successful login should reset failedLoginCount to 0"
    );
    assert!(
        locked_until.is_none(),
        "successful login should clear lockedUntil"
    );
}

#[tokio::test]
async fn password_reset_returns_200_for_missing_account() {
    set_test_ratelimits();
    let (state, _dirs) = test_state().await;
    let db = state.account_manager.db.clone();
    let app = build_app_with_state(state).await;
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/requestPasswordReset")
        .body_json(&json!({
            "email": "nobody@example.com",
        }))
        .send()
        .await;
    assert_eq!(
        resp.0.status(),
        poem::http::StatusCode::OK,
        "requestPasswordReset for missing account must return 200 OK"
    );
    // No email token should have been minted for the missing account.
    let count: i64 = cacos_pds_account::account::helpers::account::count_email_tokens_for_email(
        "nobody@example.com",
        &db,
    )
    .await
    .unwrap_or(0);
    assert_eq!(
        count, 0,
        "requestPasswordReset must not create an email_token row for missing accounts"
    );
}
