use crate::xrpc::auth_extractors::RevokeRefreshToken;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Data;

/// POST /xrpc/com.atproto.server.deleteSession
#[poem::handler]
pub async fn delete_session(auth: RevokeRefreshToken, state: Data<&SharedState>) -> ApiResult<()> {
    match state.account_manager.revoke_refresh_token(auth.id).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}
