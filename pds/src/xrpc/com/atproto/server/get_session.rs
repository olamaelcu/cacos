use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::GetSessionOutput;
use rsky_syntax::handle::INVALID_HANDLE;

/// GET /xrpc/com.atproto.server.getSession
#[poem::handler]
pub async fn get_session(
    auth: AccessStandard,
    state: Data<&SharedState>,
) -> ApiResult<Json<GetSessionOutput>> {
    let credentials = auth.access.credentials.ok_or(ApiError::InvalidRequest(
        "Missing credentials on access token".to_string(),
    ))?;
    let did = credentials
        .did
        .ok_or(ApiError::InvalidRequest("Missing did on access token".to_string()))?;
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