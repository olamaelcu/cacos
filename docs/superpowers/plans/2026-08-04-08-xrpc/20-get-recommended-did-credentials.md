# Task 20: getRecommendedDidCredentials

**Files:**
- Create: `pds/src/xrpc/com/atproto/identity/get_recommended_did_credentials.rs`
- Test: `pds/tests/identity_get_recommended_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
// pds/tests/identity_get_recommended_test.rs
use pds::xrpc::build_app;
use pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;

#[tokio::test]
async fn get_recommended_did_credentials_returns_keys() {
    let (state, _dirs) = test_state().await;
    let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
    let app = build_app(state);
    let cli = TestClient::new(app);
    let resp = cli
        .get("/xrpc/com.atproto.identity.getRecommendedDidCredentials")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    assert_eq!(body["alsoKnownAs"][0], "at://alice.test");
    assert_eq!(
        body["verificationMethods"]["atproto"].as_str().unwrap().starts_with("did:key:"),
        true
    );
    assert_eq!(body["rotationKeys"].as_array().unwrap().len(), 1);
    assert_eq!(body["services"]["atproto_pds"]["type"], "AtprotoPersonalDataServer");
}
```

Run: `cargo test -p pds --test identity_get_recommended_test`
Expected: FAIL — handler missing.

- [ ] **Step 2: Implement the handler**

Port of `the git-pinned `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a` (`rsky` crate's `src/apis/com/atproto/identity/get_recommended_did_credentials.rs`.

```rust
// pds/src/xrpc/com/atproto/identity/get_recommended_did_credentials.rs
use crate::account::helpers::account::AvailabilityFlags;
use crate::context::PDS_REPO_SIGNING_KEYPAIR;
use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::com::atproto::server::PDS_PLC_ROTATION_KEYPAIR;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Json;
use poem::State;
use rsky_crypto::utils::encode_did_key;
use rsky_lexicon::com::atproto::identity::GetRecommendedDidCredentialsResponse;
use serde_json::json;

/// GET /xrpc/com.atproto.identity.getRecommendedDidCredentials
#[poem::handler]
pub async fn get_recommended_did_credentials(
    auth: AccessStandard,
    state: State<SharedState>,
) -> ApiResult<Json<GetRecommendedDidCredentialsResponse>> {
    let requester = auth.access.credentials.unwrap().did.unwrap();
    let availability_flags = AvailabilityFlags {
        include_taken_down: Some(true),
        include_deactivated: Some(true),
    };
    let account = state
        .account_manager
        .get_account(&requester, Some(availability_flags))
        .await
        .map_err(|_| ApiError::RuntimeError)?
        .ok_or_else(|| ApiError::RuntimeError)?;

    let mut also_known_as = Vec::new();
    match account.handle {
        None => {}
        Some(res) => {
            also_known_as.push("at://".to_string() + res.as_str());
        }
    }

    let signing_key = encode_did_key(&PDS_REPO_SIGNING_KEYPAIR.public_key());
    let verification_methods = json!({
        "atproto": signing_key
    });

    let rotation_key = encode_did_key(&PDS_PLC_ROTATION_KEYPAIR.public_key());
    let rotation_keys = vec![rotation_key];

    let services = json!({
        "atproto_pds": {
            "type": "AtprotoPersonalDataServer",
            "endpoint": state.config.service.public_url
        }
    });
    let response = GetRecommendedDidCredentialsResponse {
        also_known_as,
        verification_methods,
        rotation_keys,
        services,
    };
    Ok(Json(response))
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pds --test identity_get_recommended_test`
Expected: PASS (1 test).

- [ ] **Step 4: Commit**

```bash
git add pds/src/xrpc/com/atproto/identity/get_recommended_did_credentials.rs pds/tests/identity_get_recommended_test.rs
git commit -m "feat(identity): getRecommendedDidCredentials"
```
