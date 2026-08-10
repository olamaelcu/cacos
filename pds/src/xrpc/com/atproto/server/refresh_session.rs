use crate::account::helpers::account::AvailabilityFlags;
use crate::xrpc::auth_extractors::Refresh;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::RefreshSessionOutput;
use rsky_syntax::handle::INVALID_HANDLE;

async fn inner_refresh_session(
    auth: Refresh,
    state: &SharedState,
) -> Result<RefreshSessionOutput, ApiError> {
    let credentials = auth.access.credentials.ok_or(ApiError::InvalidRequest(
        "Missing credentials on refresh token".to_string(),
    ))?;
    let did = credentials.did.ok_or(ApiError::InvalidRequest(
        "Missing did on refresh token".to_string(),
    ))?;
    let token_id = credentials.token_id.ok_or(ApiError::InvalidRequest(
        "Missing token id on refresh token".to_string(),
    ))?;
    let user = state
        .account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await
        .map_err(|e| {
            tracing::error!("{e:?}");
            ApiError::RuntimeError
        })?;

    if let Some(user) = user {
        if user.takedown_ref.is_some() {
            return Err(ApiError::AccountTakendown);
        }
        let rotated = state
            .account_manager
            .rotate_refresh_token(&token_id)
            .await
            .map_err(|e| {
                tracing::error!("{e:?}");
                ApiError::RuntimeError
            })?;
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
    state: Data<&SharedState>,
) -> ApiResult<Json<RefreshSessionOutput>> {
    match inner_refresh_session(auth, state.0).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => Err(error),
    }
}
