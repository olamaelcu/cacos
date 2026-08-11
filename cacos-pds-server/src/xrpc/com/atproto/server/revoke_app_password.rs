use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::RevokeAppPasswordInput;

/// POST /xrpc/com.atproto.server.revokeAppPassword
#[poem::handler]
pub async fn revoke_app_password(
    body: Json<RevokeAppPasswordInput>,
    auth: AccessStandard,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    let RevokeAppPasswordInput { name } = body.0;
    let credentials = auth.access.credentials.ok_or(ApiError::InvalidRequest(
        "Missing credentials on access token".to_string(),
    ))?;
    let did = credentials.did.ok_or(ApiError::InvalidRequest(
        "Missing did on access token".to_string(),
    ))?;

    match state.account_manager.revoke_app_password(did, name).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
