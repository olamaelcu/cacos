//! `com.atproto.server.deleteAccount` handler.

use cacos_pds_account::account::EmailTokenPurpose;
use cacos_pds_account::account::helpers::account::{AccountStatus, AvailabilityFlags};
use crate::xrpc::auth_extractors::RequireAccountAdmin;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::DeleteAccountInput;

async fn inner_delete_account(
    body: DeleteAccountInput,
    state: &SharedState,
) -> Result<(), ApiError> {
    let DeleteAccountInput {
        did,
        password,
        token,
    } = body;
    let account = state
        .account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    if account.is_some() {
        let valid_pass = state
            .account_manager
            .verify_account_password(&did, &password)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        if !valid_pass {
            return Err(ApiError::InvalidLogin);
        }
        state
            .account_manager
            .assert_valid_email_token(&did, EmailTokenPurpose::DeleteAccount, &token)
            .await
            .map_err(|_| ApiError::RuntimeError)?;

        state
            .actor_store
            .destroy(&did, state.blobstore.clone())
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        state
            .account_manager
            .delete_account(&did)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        let mut sequencer_clone = state
            .sequencer
            .sequencer
            .read()
            .expect("sequencer lock poisoned")
            .clone();
        let (active, status) = {
            let s: AccountStatus = AccountStatus::Deleted;
            s.into()
        };
        sequencer_clone
            .sequence_account_evt(did.clone(), active, status)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        sequencer_clone
            .delete_all_for_user(&did)
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        Ok(())
    } else {
        tracing::error!("account not found");
        Err(ApiError::RuntimeError)
    }
}

/// POST /xrpc/com.atproto.server.deleteAccount (admin token, per lexicon)
#[poem::handler]
pub async fn delete_account(
    body: Json<DeleteAccountInput>,
    _auth: RequireAccountAdmin,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    match inner_delete_account(body.0, state.0).await {
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}
