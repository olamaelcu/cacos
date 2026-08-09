use crate::account::helpers::account::AvailabilityFlags;
use crate::account::{AccountManager, ConfirmEmailOpts};
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::ConfirmEmailInput;

async fn inner_confirm_email(
    body: ConfirmEmailInput,
    auth: AccessFull,
    account_manager: &AccountManager,
) -> Result<(), ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();

    let user = account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await
        .map_err(|e| {
            tracing::error!("Error: {e}");
            ApiError::RuntimeError
        })?;
    if let Some(user) = user {
        if let Some(user_email) = user.email {
            let ConfirmEmailInput { token, email } = body;
            if user_email != email.to_lowercase() {
                return Err(ApiError::InvalidEmail);
            }
            account_manager
                .confirm_email(ConfirmEmailOpts {
                    did: &did,
                    token: &token,
                })
                .await
                .map_err(|e| {
                    tracing::error!("Error: {e}");
                    ApiError::RuntimeError
                })?;
            Ok(())
        } else {
            Err(ApiError::InvalidRequest("Missing Email".to_string()))
        }
    } else {
        Err(ApiError::AccountNotFound)
    }
}

/// POST /xrpc/com.atproto.server.confirmEmail
#[poem::handler]
pub async fn confirm_email(
    body: Json<ConfirmEmailInput>,
    auth: AccessFull,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    match inner_confirm_email(body.0, auth, &state.account_manager).await {
        Ok(()) => Ok(()),
        Err(error) => Err(error),
    }
}