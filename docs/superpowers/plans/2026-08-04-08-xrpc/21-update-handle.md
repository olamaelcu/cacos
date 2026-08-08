# Task 21: updateHandle

**Files:**
- Create: `pds/src/xrpc/com/atproto/identity/update_handle.rs`
- Test: `pds/tests/identity_update_handle_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// pds/tests/identity_update_handle_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

#[tokio::test]
async fn update_handle_taken_handle_rejected() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    create_test_account(&state, "did:plc:bob", "bob.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .post("/xrpc/com.atproto.identity.updateHandle")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "handle": "bob.test" }))
        .send()
        .await;
    // bob.test is owned by a different account → the handler bails before
    // touching the PLC client
    assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_handle_self_handle_noop_succeeds() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    // same handle → account.did == requester branch → Ok(()) without PLC
    let resp = cli
        .post("/xrpc/com.atproto.identity.updateHandle")
        .header("Authorization", format!("Bearer {access}"))
        .body_json(&json!({ "handle": "alice.test" }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
}
```

> **Test note:** changing to a brand-new handle hits `plc_client.update_handle` (mock in tests) then `account_manager.update_handle`. The "taken" and "no-op" branches avoid the PLC path entirely and are the hermetic tests; the new-handle branch is covered by the mock.

Run: `cargo test -p pds --test identity_update_handle_test`
Expected: FAIL — handler missing.

- [ ] **Step 2: Implement `update_handle.rs`**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/identity/update_handle.rs`.

```rust
// pds/src/xrpc/com/atproto/identity/update_handle.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::handle::{normalize_and_validate_handle, HandleValidationOpts};
use crate::xrpc::auth_extractors::AccessStandardCheckTakedown;
use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use anyhow::{bail, Result};
use poem::web::Json;
use poem::State;
use rsky_lexicon::com::atproto::identity::UpdateHandleInput;

async fn inner_update_handle(
    body: UpdateHandleInput,
    auth: AccessStandardCheckTakedown,
    state: &SharedState,
) -> Result<()> {
    let UpdateHandleInput { handle } = body;
    let requester = auth.access.credentials.unwrap().did.unwrap();

    let handle = normalize_and_validate_handle(
        HandleValidationOpts {
            handle,
            did: Some(requester.clone()),
            allow_reserved: None,
        },
        &state.config.identity.service_handle_domains,
    )
    .await
    .map_err(|e| anyhow::Error::msg(format!("{:?}: {}", e.kind, e.message)))?;

    let account = state
        .account_manager
        .get_account(
            &handle,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await?;

    match account {
        Some(account) if account.did != requester => bail!("Handle already taken: {handle}"),
        Some(_) => (),
        None => {
            state
                .plc_client
                .update_handle(
                    &requester,
                    &PDS_PLC_ROTATION_KEYPAIR.secret_key(),
                    &handle,
                )
                .await?;
            state.account_manager.update_handle(&requester, &handle).await?;
        }
    }
    let mut lock = state.sequencer.sequencer.write().await;
    if let Err(error) = lock
        .sequence_identity_evt(requester.clone(), Some(handle.clone()))
        .await
    {
        tracing::error!("Error: {}; DID: {}; Handle: {}", error, &requester, &handle);
    };
    if let Err(error) = lock
        .sequence_handle_update(requester.clone(), handle.clone())
        .await
    {
        tracing::error!("Error: {}; DID: {}; Handle: {}", error, &requester, &handle);
    };
    Ok(())
}

/// POST /xrpc/com.atproto.identity.updateHandle
#[poem::handler]
pub async fn update_handle(
    body: Json<UpdateHandleInput>,
    auth: AccessStandardCheckTakedown,
    state: State<SharedState>,
) -> ApiResult<()> {
    match inner_update_handle(body.0, auth, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test identity_update_handle_test`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/identity/update_handle.rs pds/tests/identity_update_handle_test.rs
git commit -m "feat(identity): updateHandle"
```
