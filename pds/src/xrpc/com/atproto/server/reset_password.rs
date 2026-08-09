use crate::account::ResetPasswordOpts;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::ResetPasswordInput;

/// POST /xrpc/com.atproto.server.resetPassword
#[poem::handler]
pub async fn reset_password(
    body: Json<ResetPasswordInput>,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    let ResetPasswordInput { token, password } = body.0;
    match state
        .account_manager
        .reset_password(ResetPasswordOpts { token, password })
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}