# Task 6: Sessions — createSession, refreshSession, deleteSession, getSession

**Files:**
- Create: `pds/src/xrpc/com/atproto/server/create_session.rs`
- Create: `pds/src/xrpc/com/atproto/server/refresh_session.rs`
- Create: `pds/src/xrpc/com/atproto/server/delete_session.rs`
- Create: `pds/src/xrpc/com/atproto/server/get_session.rs`
- Test: `pds/tests/server_sessions_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/server_sessions_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn create_session_with_password() {
    let (state, _dirs) = test_state().await;
    let (_access, _refresh) =
        create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.createSession")
        .body_json(&json!({ "identifier": "alice.test", "password": "password123" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["handle"], "alice.test");
    assert_eq!(body["did"], "did:plc:alice");
    assert!(body["accessJwt"].as_str().is_some());
    assert!(body["refreshJwt"].as_str().is_some());
}

#[tokio::test]
async fn create_session_bad_password_is_invalid_login() {
    let (state, _dirs) = test_state().await;
    let (_access, _refresh) =
        create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.createSession")
        .body_json(&json!({ "identifier": "bob.test", "password": "wrong" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["error"], "InvalidLogin");
}

#[tokio::test]
async fn refresh_session_rotates_token() {
    let (state, _dirs) = test_state().await;
    let (_access, refresh) =
        create_test_account(&state, "did:plc:carol", "carol.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.refreshSession")
        .header("Authorization", format!("Bearer {refresh}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:carol");
    assert!(body["accessJwt"].as_str().is_some());
    assert_ne!(body["refreshJwt"].as_str().unwrap(), refresh);
}

#[tokio::test]
async fn get_session_returns_account() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) =
        create_test_account(&state, "did:plc:dan", "dan.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.server.getSession")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["did"], "did:plc:dan");
    assert_eq!(body["handle"], "dan.test");
}

#[tokio::test]
async fn delete_session_revokes_refresh_token() {
    let (state, _dirs) = test_state().await;
    let (_access, refresh) =
        create_test_account(&state, "did:plc:erin", "erin.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.server.deleteSession")
        .header("Authorization", format!("Bearer {refresh}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    // the revoked refresh token can no longer refresh
    let resp = cli
        .post("/xrpc/com.atproto.server.refreshSession")
        .header("Authorization", format!("Bearer {refresh}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
}
```

Run: `cargo test -p pds --test server_sessions_test`
Expected: FAIL — handlers missing.

- [ ] **Step 2: Implement `create_session.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/create_session.rs`.

```rust
// pds/src/xrpc/com/atproto/server/create_session.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::AccountManager;
use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;
use rsky_lexicon::com::atproto::server::{CreateSessionInput, CreateSessionOutput};
use rsky_syntax::handle::INVALID_HANDLE;

async fn inner_create_session(
    body: CreateSessionInput,
    account_manager: &AccountManager,
) -> Result<CreateSessionOutput, ApiError> {
    let CreateSessionInput {
        password,
        identifier,
    } = body;
    let identifier = identifier.to_lowercase();

    let user = if identifier.contains('@') {
        account_manager
            .get_account_by_email(
                &identifier,
                Some(AvailabilityFlags {
                    include_deactivated: Some(true),
                    include_taken_down: Some(true),
                }),
            )
            .await
    } else {
        account_manager
            .get_account(
                &identifier,
                Some(AvailabilityFlags {
                    include_deactivated: Some(true),
                    include_taken_down: Some(true),
                }),
            )
            .await
    };
    if let Ok(Some(user)) = user {
        let mut app_password_name: Option<String> = None;

        let valid_account_pass = match account_manager
            .verify_account_password(&user.did, &password)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                tracing::error!("{e:?}");
                return Err(ApiError::RuntimeError);
            }
        };
        if !valid_account_pass {
            match account_manager
                .verify_app_password(&user.did, &password)
                .await
            {
                Ok(res) => {
                    app_password_name = res;
                }
                Err(e) => {
                    tracing::error!("{e:?}");
                    return Err(ApiError::RuntimeError);
                }
            }
            if app_password_name.is_none() {
                return Err(ApiError::InvalidLogin);
            }
        }
        if user.takedown_ref.is_some() {
            return Err(ApiError::AccountTakendown);
        }
        let (access_jwt, refresh_jwt);
        match account_manager
            .create_session(user.did.clone(), app_password_name)
            .await
        {
            Ok(res) => {
                (access_jwt, refresh_jwt) = res;
            }
            Err(e) => {
                tracing::error!("{e:?}");
                return Err(ApiError::RuntimeError);
            }
        }
        Ok(CreateSessionOutput {
            did: user.did,
            did_doc: None,
            handle: user.handle.unwrap_or(INVALID_HANDLE.to_string()),
            email: user.email,
            email_confirmed: Some(user.email_confirmed_at.is_some()),
            access_jwt,
            refresh_jwt,
        })
    } else {
        Err(ApiError::InvalidLogin)
    }
}

/// POST /xrpc/com.atproto.server.createSession
#[poem::handler]
pub async fn create_session(
    body: Json<CreateSessionInput>,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<Json<CreateSessionOutput>> {
    match inner_create_session(body.0, &state.account_manager).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => Err(error),
    }
}
```

- [ ] **Step 3: Implement `refresh_session.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/refresh_session.rs`.

```rust
// pds/src/xrpc/com/atproto/server/refresh_session.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::account::AccountManager;
use crate::xrpc::auth_extractors::{Credentials, Refresh};
use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;
use rsky_lexicon::com::atproto::server::RefreshSessionOutput;
use rsky_syntax::handle::INVALID_HANDLE;

async fn inner_refresh_session(
    auth: Refresh,
    account_manager: &AccountManager,
) -> Result<RefreshSessionOutput, ApiError> {
    let Credentials { did, token_id, .. } = auth.access.credentials.unwrap();
    let did = did.unwrap();
    let token_id = token_id.unwrap();
    let user = account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(user) = user {
        if user.takedown_ref.is_some() {
            return Err(ApiError::AccountTakendown);
        }
        let rotated = account_manager
            .rotate_refresh_token(&token_id)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        if let Some(rotated) = rotated {
            Ok(RefreshSessionOutput {
                handle: user.handle.unwrap_or(INVALID_HANDLE.to_string()),
                did,
                did_doc: None,
                access_jwt: rotated.0,
                refresh_jwt: rotated.1,
            })
        } else {
            Err(ApiError::ExpiredToken)
        }
    } else {
        Err(ApiError::AccountNotFound)
    }
}

/// POST /xrpc/com.atproto.server.refreshSession
#[poem::handler]
pub async fn refresh_session(
    auth: Refresh,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<Json<RefreshSessionOutput>> {
    match inner_refresh_session(auth, &state.account_manager).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => Err(error),
    }
}
```

- [ ] **Step 4: Implement `delete_session.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/delete_session.rs`.

```rust
// pds/src/xrpc/com/atproto/server/delete_session.rs
use crate::xrpc::auth_extractors::RevokeRefreshToken;
use crate::xrpc::{ApiError, ApiResult};

/// POST /xrpc/com.atproto.server.deleteSession
#[poem::handler]
pub async fn delete_session(
    auth: RevokeRefreshToken,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<()> {
    match state.account_manager.revoke_refresh_token(auth.id).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 5: Implement `get_session.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/server/get_session.rs`.

```rust
// pds/src/xrpc/com/atproto/server/get_session.rs
use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;
use rsky_lexicon::com::atproto::server::GetSessionOutput;
use rsky_syntax::handle::INVALID_HANDLE;

/// GET /xrpc/com.atproto.server.getSession
#[poem::handler]
pub async fn get_session(
    auth: AccessStandard,
    state: poem::State<crate::xrpc::SharedState>,
) -> ApiResult<Json<GetSessionOutput>> {
    let did = auth.access.credentials.unwrap().did.unwrap();
    match state.account_manager.get_account(&did, None).await {
        Ok(Some(user)) => Ok(Json(GetSessionOutput {
            handle: user.handle.unwrap_or(INVALID_HANDLE.to_string()),
            did: user.did,
            email: user.email,
            did_doc: None,
            email_confirmed: Some(user.email_confirmed_at.is_some()),
        })),
        _ => Err(ApiError::AccountNotFound),
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p pds --test server_sessions_test`
Expected: PASS (5 tests).

- [ ] **Step 7: Commit**

```bash
git add pds/src/xrpc/com/atproto/server/create_session.rs pds/src/xrpc/com/atproto/server/refresh_session.rs pds/src/xrpc/com/atproto/server/delete_session.rs pds/src/xrpc/com/atproto/server/get_session.rs pds/tests/server_sessions_test.rs
git commit -m "feat(server): session handlers (create/refresh/delete/get)"
```
