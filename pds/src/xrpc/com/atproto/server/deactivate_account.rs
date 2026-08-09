//! `com.atproto.server.deactivateAccount` handler.

use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::DeactivateAccountInput;

/// POST /xrpc/com.atproto.server.deactivateAccount
#[poem::handler]
pub async fn deactivate_account(
    body: Json<DeactivateAccountInput>,
    auth: AccessFull,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    let did = auth
        .access
        .credentials
        .ok_or(ApiError::InvalidRequest(
            "Missing credentials on access token".to_string(),
        ))?
        .did
        .ok_or(ApiError::InvalidRequest(
            "Missing did on access token".to_string(),
        ))?;
    let DeactivateAccountInput { delete_after } = body.0;
    match state
        .account_manager
        .deactivate_account(&did, delete_after)
        .await
    {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
