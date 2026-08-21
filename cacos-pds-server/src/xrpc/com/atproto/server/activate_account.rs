//! `com.atproto.server.activateAccount` handler.

use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::com::atproto::server::assert_valid_did_documents_for_service;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use cacos_pds_account::account::AccountManager;
use cacos_pds_account::account::helpers::account::AvailabilityFlags;
use poem::web::Data;
use rsky_syntax::handle::INVALID_HANDLE;
use tracing_unwrap::ResultExt;

async fn inner_activate_account(auth: AccessFull, state: &SharedState) -> Result<(), ApiError> {
    let requester = auth
        .access
        .credentials
        .ok_or(ApiError::InvalidRequest(
            "Missing credentials on access token".to_string(),
        ))?
        .did
        .ok_or(ApiError::InvalidRequest(
            "Missing did on access token".to_string(),
        ))?;
    assert_valid_did_documents_for_service(
        requester.clone(),
        state.plc_client.as_ref(),
        &state.config.service,
    )
    .await
    .map_err(|_| ApiError::RuntimeError)?;

    let account = state
        .account_manager
        .get_account(
            &requester,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        state
            .account_manager
            .activate_account(&requester)
            .await
            .map_err(|_| ApiError::RuntimeError)?;

        let actor_store = state
            .actor_store
            .read(requester.clone(), state.blobstore.clone())
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        let sync_data = actor_store
            .get_sync_event_data()
            .await
            .map_err(|_| ApiError::RuntimeError)?;

        let status = state
            .account_manager
            .get_account_status(&requester)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        let mut sequencer_clone = state
            .sequencer
            .sequencer
            .read()
            .expect_or_log("sequencer lock poisoned")
            .clone();
        let (active, status_lex) = status.into();
        sequencer_clone
            .sequence_account_evt(requester.clone(), active, status_lex)
            .await
            .map_err(|_| ApiError::RuntimeError)?;

        let handle = account.handle.unwrap_or(INVALID_HANDLE.to_string());
        sequencer_clone
            .sequence_identity_evt(requester.clone(), Some(handle))
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        sequencer_clone
            .sequence_sync_evt(requester, sync_data.rev, sync_data.blocks)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        Ok(())
    } else {
        tracing::error!("User not found");
        Err(ApiError::RuntimeError)
    }
}

/// POST /xrpc/com.atproto.server.activateAccount
#[poem::handler]
pub async fn activate_account(auth: AccessFull, state: Data<&SharedState>) -> ApiResult<()> {
    match inner_activate_account(auth, state.0).await {
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}

// `AccountManager` is used by the test path indirectly through
// `state.account_manager`. The import keeps this file self-documenting
// alongside the other lifecycle handlers and silences the unused-import
// lint when the test_utils do not exercise it directly.
#[allow(dead_code)]
fn _ensure_account_manager_in_scope(_am: AccountManager) {}
