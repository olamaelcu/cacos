# Task 9: App passwords — createAppPassword, listAppPasswords, revokeAppPassword

**Files:**
- Create: `pds/src/xrpc/com/atproto/server/create_app_password.rs`
- Create: `pds/src/xrpc/com/atproto/server/list_app_passwords.rs`
- Create: `pds/src/xrpc/com/atproto/server/revoke_app_password.rs`
- Test: `pds/tests/server_app_password_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/server_app_password_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn create_list_revoke_app_password() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:a", "a.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/com.atproto.server.createAppPassword")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "name": "My App" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let created: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(created["name"], "My App");
    assert_eq!(created["password"].as_str().unwrap().len(), 19);

    let resp = cli
        .get("/xrpc/com.atproto.server.listAppPasswords")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let listed: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(listed["passwords"].as_array().unwrap().len(), 1);

    let resp = cli
        .post("/xrpc/com.atproto.server.revokeAppPassword")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "name": "My App" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);

    let resp = cli
        .get("/xrpc/com.atproto.server.listAppPasswords")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    let listed: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert!(listed["passwords"].as_array().unwrap().is_empty());
}
```

Run: `cargo test -p pds --test server_app_password_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement the handlers**

```rust
// pds/src/xrpc/com/atproto/server/create_app_password.rs
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::{CreateAppPasswordInput, CreateAppPasswordOutput};

/// POST /xrpc/com.atproto.server.createAppPassword
#[poem::handler]
pub async fn create_app_password(
    body: Json<CreateAppPasswordInput>,
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<Json<CreateAppPasswordOutput>> {
    let CreateAppPasswordInput { name } = body.0;
    match state
        .account_manager
        .create_app_password(auth.access.credentials.unwrap().did.unwrap(), name)
        .await
    {
        Ok(app_password) => Ok(Json(app_password)),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/list_app_passwords.rs
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::{AppPassword, ListAppPasswordsOutput};

/// GET /xrpc/com.atproto.server.listAppPasswords
#[poem::handler]
pub async fn list_app_passwords(
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<Json<ListAppPasswordsOutput>> {
    let did = auth.access.credentials.unwrap().did.unwrap();
    match state.account_manager.list_app_passwords(&did).await {
        Ok(passwords) => {
            let passwords: Vec<AppPassword> = passwords
                .into_iter()
                .map(|password| AppPassword {
                    name: password.0,
                    created_at: password.1,
                })
                .collect();
            Ok(Json(ListAppPasswordsOutput { passwords }))
        }
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

```rust
// pds/src/xrpc/com/atproto/server/revoke_app_password.rs
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::server::RevokeAppPasswordInput;

/// POST /xrpc/com.atproto.server.revokeAppPassword
#[poem::handler]
pub async fn revoke_app_password(
    body: Json<RevokeAppPasswordInput>,
    auth: AccessFull,
    state: State<SharedState>,
) -> ApiResult<()> {
    let RevokeAppPasswordInput { name } = body.0;
    let requester = auth.access.credentials.unwrap().did.unwrap();

    match state.account_manager.revoke_app_password(requester, name).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test server_app_password_test`
Expected: PASS (1 test).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/server/create_app_password.rs pds/src/xrpc/com/atproto/server/list_app_passwords.rs pds/src/xrpc/com/atproto/server/revoke_app_password.rs pds/tests/server_app_password_test.rs
git commit -m "feat(server): app password handlers"
```
