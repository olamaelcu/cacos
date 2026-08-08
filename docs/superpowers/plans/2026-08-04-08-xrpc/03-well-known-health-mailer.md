# Task 3: well_known + health wiring + mailer

**Files:**
- Create: `pds/src/xrpc/well_known.rs`
- Modify: `pds/src/xrpc/health.rs` (add `use` for state-based account lookup if needed)
- Create: `pds/src/mailer.rs`
- Test: `pds/tests/well_known_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/well_known_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;

#[tokio::test]
async fn well_known_returns_did_for_local_account() {
    let (state, _dirs) = test_state().await;
    create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/.well-known/atproto-did")
        .header("Host", "alice.test")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    assert_eq!(resp.0.into_body().into_string().await.unwrap(), "did:plc:alice");
}

#[tokio::test]
async fn well_known_404_for_unknown_handle() {
    let (state, _dirs) = test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/.well-known/atproto-did")
        .header("Host", "nobody.test")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
    assert_eq!(resp.0.into_body().into_string().await.unwrap(), "User not found");
}

#[tokio::test]
async fn well_known_404_for_unsupported_host() {
    let (state, _dirs) = test_state().await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/.well-known/atproto-did")
        .header("Host", "evil.example.com")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
}
```

Run: `cargo test -p pds --test well_known_test`
Expected: FAIL — `well_known` module/function missing.

- [ ] **Step 2: Implement `xrpc/well_known.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/well_known.rs`. The `Host` header check is identical; the account lookup uses `SharedState`.

```rust
// pds/src/xrpc/well_known.rs
use crate::xrpc::SharedState;
use poem::http::StatusCode;
use poem::Request;
use poem::Response;
use poem::Result;

/// GET /.well-known/atproto-did — maps the Host header to a handle and answers
/// with the account DID, or 404 "User not found".
#[poem::handler]
pub async fn well_known(state: poem::State<SharedState>, req: &Request) -> Result<Response> {
    let handle = match req.headers().get("host").and_then(|v| v.to_str().ok()) {
        Some(h) => h.to_string(),
        None => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .finish())
        }
    };
    let supported_handle = state
        .config
        .identity
        .service_handle_domains
        .iter()
        .any(|host| handle.ends_with(host.as_str()) || handle == host[1..]);
    if !supported_handle {
        return Ok(not_found());
    }
    match state
        .account_manager
        .get_account(&handle, None)
        .await
    {
        Ok(Some(user)) => Ok(Response::builder()
            .status(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(user.did)),
        Ok(None) => Ok(not_found()),
        Err(_) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .content_type("text/plain; charset=utf-8")
            .body("Internal Server Error")),
    }
}

fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .content_type("text/plain; charset=utf-8")
        .body("User not found")
}
```

- [ ] **Step 3: Implement `pds/src/mailer.rs` — logging no-op mailer**

The reference mailer (`the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/mailer/mod.rs`) sends via mailgun. cacos logs instead, keeping the same free-function signatures so handler ports stay 1:1.

```rust
// pds/src/mailer.rs
use anyhow::Result;
use std::collections::HashMap;

pub struct MailOpts {
    pub to: String,
    pub subject: String,
    pub template: String,
    pub template_vars: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdentifierAndTokenParams {
    pub identifier: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TokenParam {
    pub token: String,
}

pub async fn send_template(opts: MailOpts) -> Result<()> {
    // No SMTP provider in cacos yet; surface through tracing so flows are
    // observable and tests can assert tokens were minted via account_manager.
    tracing::info!(
        to = %opts.to,
        subject = %opts.subject,
        template = %opts.template,
        "mailer: template mail (not sent)"
    );
    Ok(())
}

pub async fn send_reset_password(to: String, params: IdentifierAndTokenParams) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("identifier".to_string(), params.identifier);
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Password Reset Requested".to_string(),
        template: "reset password".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_account_delete(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Account Deletion Requested".to_string(),
        template: "delete account".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_confirm_email(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Email Confirmation".to_string(),
        template: "confirm email".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_update_email(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "Email Update Requested".to_string(),
        template: "email update".to_string(),
        template_vars,
    })
    .await
}

pub async fn send_plc_operation(to: String, params: TokenParam) -> Result<()> {
    let mut template_vars = HashMap::new();
    template_vars.insert("token".to_string(), params.token);
    send_template(MailOpts {
        to,
        subject: "PLC Update Operation Requested".to_string(),
        template: "plc operation".to_string(),
        template_vars,
    })
    .await
}

pub mod moderation {
    use super::MailOpts;

    pub struct HtmlMailOpts {
        pub to: String,
        pub subject: String,
        pub html: String,
    }

    pub struct ModerationMailer;

    impl ModerationMailer {
        pub async fn send_html(opts: HtmlMailOpts) -> anyhow::Result<()> {
            tracing::info!(
                to = %opts.to,
                subject = %opts.subject,
                "mailer: moderation html mail (not sent)"
            );
            Ok(())
        }
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pds --test well_known_test`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add pds/src/xrpc/well_known.rs pds/src/mailer.rs pds/tests/well_known_test.rs
git commit -m "feat(xrpc): well-known atproto-did + logging mailer"
```
