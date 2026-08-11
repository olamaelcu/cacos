//! Integration test for the production [`ActorStoreRemoteCreateAccount`]
//! impl. Pins the seam so the headless-consent "create account" flow
//! produces a real account, a real actor store, and a real `did:plc`,
//! not the placeholder `Internal("wiring pending phase-e")` error.

use cacos_pds_oauth::remote_create_account::ActorStoreRemoteCreateAccount;
use cacos_pds_oauth::remote_create_account::{CreateAccountInput, RemoteCreateAccount};
use cacos_pds_server::xrpc::test_utils::test_state;

fn input() -> CreateAccountInput {
    CreateAccountInput {
        rqid: "rq-test".into(),
        request_uri: "urn:ietf:params:oauth:request_uri:rq-test".into(),
        client_id: "https://app.example.com/client".into(),
        device_id: "dev-1".into(),
        handle: "alice.test".into(),
        email: "alice@example.com".into(),
        password: "password123".into(),
        invite_code: None,
    }
}

#[tokio::test]
async fn actor_store_remote_create_account_rejects_empty_handle_without_mutating_state() {
    let (state, _dirs) = test_state().await;
    let impl_ = ActorStoreRemoteCreateAccount::new(
        state.account_manager.clone(),
        state.actor_store.clone(),
        state.plc_client.clone(),
        state.blobstore.clone(),
        state.sequencer.clone(),
    );
    let mut invalid_input = input();
    invalid_input.handle = "  ".into();

    let result = impl_.create_account(invalid_input).await;

    assert!(matches!(
        result,
        Err(cacos_pds_oauth::remote_create_account::CreateAccountError::InvalidInput(message))
            if message == "handle must not be empty"
    ));
    assert!(
        state
            .account_manager
            .get_account("alice.test", None)
            .await
            .expect("account lookup must not error")
            .is_none()
    );
}

#[tokio::test]
async fn actor_store_remote_create_account_produces_did_and_account() {
    let (state, _dirs) = test_state().await;
    let impl_ = ActorStoreRemoteCreateAccount::new(
        state.account_manager.clone(),
        state.actor_store.clone(),
        state.plc_client.clone(),
        state.blobstore.clone(),
        state.sequencer.clone(),
    );
    let result = impl_.create_account(input()).await;
    let did = result.expect("create_account should produce a did:plc, not a stub error");
    assert!(!did.is_empty(), "did must be non-empty");
    assert!(
        did.starts_with("did:plc:"),
        "did must be a did:plc, got: {did}"
    );
    let account = state
        .account_manager
        .get_account("alice.test", None)
        .await
        .expect("account lookup must not error")
        .expect("account must be persisted after create_account");
    assert_eq!(account.did, did);
    assert_eq!(account.handle.as_deref(), Some("alice.test"));
}
