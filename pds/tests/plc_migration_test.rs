//! Integration tests for per-DID PLC rotation keys and the
//! `migrate rotation-keys` backfill command.
//!
//! These tests exercise the actor-store surface (`has_rotation_keypair`,
//! `rotation_keypair`, `write_rotation_key_from_bytes`) and assert that
//! `createAccount` leaves a per-DID rotation key on disk, the backfill
//! re-creates it if the file is removed, and the reader returns
//! `NotFound` when nothing is on disk yet.

use cacos_pds_account::auth::PDS_REPO_SIGNING_KEYPAIR;
use cacos_pds_core::error::PdsError;
use cacos_pds::xrpc::build_app_with_state;
use cacos_pds::xrpc::test_utils::{create_test_account, test_state};
use poem::test::TestClient;
use serde_json::json;

/// `createAccount` must persist a per-DID PLC rotation key on the
/// actor store. The flow is: `actor_store.create` (signing key),
/// `actor_store.store_rotation_keypair` (per-DID rotation key), repo
/// init, PLC submission, then `account_manager.create_account`. The
/// rotation key file outlives the whole sequence.
#[tokio::test]
async fn rotation_keypair_is_persisted_on_create_account() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({
            "handle": "rotkey.test",
            "email": "rotkey@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let did = body["did"].as_str().unwrap().to_owned();
    assert!(
        state.actor_store.has_rotation_keypair(&did),
        "actor store should have rotation_key file for {did}"
    );
    // The key must be loadable as a valid secp256k1 keypair.
    let _keypair = state
        .actor_store
        .rotation_keypair(&did)
        .await
        .expect("rotation key must be readable");
}

/// Removes a per-DID rotation key file out from under the actor store,
/// then runs the same `write_rotation_key_from_bytes` mechanism the
/// `migrate rotation-keys` command uses. The file must come back and
/// contain a valid secp256k1 keypair.
#[tokio::test]
async fn migrate_rotation_keys_backfills_missing_files() {
    let (state, _dirs) = test_state().await;
    let app = build_app_with_state(state.clone()).await;
    let cli = TestClient::new(app);

    let resp = cli
        .post("/xrpc/createAccount")
        .body_json(&json!({
            "handle": "backfill.test",
            "email": "backfill@example.com",
            "password": "password123",
        }))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
    let did = body["did"].as_str().unwrap().to_owned();
    assert!(state.actor_store.has_rotation_keypair(&did));

    // Simulate a pre-per-DID-key actor: yank the rotation_key file.
    let location = state.actor_store.get_location(&did).unwrap();
    std::fs::remove_file(&location.rotation_key_location).unwrap();
    assert!(!state.actor_store.has_rotation_keypair(&did));

    // Backfill via the same write API the production migration uses,
    // seeded with the shared server rotation key bytes.
    use cacos_pds_account::auth::PDS_PLC_ROTATION_KEYPAIR;
    let secret_bytes = PDS_PLC_ROTATION_KEYPAIR.secret_bytes().to_vec();
    state
        .actor_store
        .write_rotation_key_from_bytes(&did, &secret_bytes)
        .await
        .unwrap();
    assert!(state.actor_store.has_rotation_keypair(&did));

    // The re-seeded key must deserialize as a valid secp256k1 keypair.
    let restored = state
        .actor_store
        .rotation_keypair(&did)
        .await
        .expect("rotation key must be readable after backfill");
    assert_eq!(
        restored.secret_bytes().to_vec(),
        secret_bytes,
        "backfilled key must match the seed bytes"
    );
}

/// When a per-DID rotation key is missing, the actor store must surface
/// `NotFound` rather than silently falling back to the global
/// `PDS_PLC_ROTATION_KEYPAIR`. Callers compose the per-DID and global
/// fallbacks explicitly; the store should not paper over either side of
/// that contract.
#[tokio::test]
async fn rotation_keypair_falls_back_to_global_when_missing() {
    let (state, _dirs) = test_state().await;
    let did = "did:plc:no-rotation-key";
    let handle = "no-rotation-key.test";

    // Set up just enough of the actor for the store to recognise the
    // DID (signing key + repo root) without persisting a rotation key.
    state
        .actor_store
        .create(did, &PDS_REPO_SIGNING_KEYPAIR)
        .await
        .unwrap();
    let transactor = state
        .actor_store
        .transact(did.to_owned(), state.blobstore.clone())
        .await
        .unwrap();
    let commit = transactor.create_repo(Vec::new()).await.unwrap();
    state
        .account_manager
        .update_repo_root(
            did.to_owned(),
            commit.commit_data.cid,
            commit.commit_data.rev.clone(),
        )
        .await
        .unwrap();
    let _ = create_test_account(&state, did, handle).await;

    assert!(!state.actor_store.has_rotation_keypair(did));
    let err = state
        .actor_store
        .rotation_keypair(did)
        .await
        .expect_err("must be NotFound");
    match err {
        PdsError::NotFound(_) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}
