use crate::account::EmailTokenPurpose;
use crate::account::helpers::account::AvailabilityFlags;
use crate::mailer;
use crate::mailer::IdentifierAndTokenParams;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::RequestPasswordResetInput;

async fn inner_request_password_reset(
    body: RequestPasswordResetInput,
    state: &SharedState,
) -> Result<(), ApiError> {
    let RequestPasswordResetInput { email } = body;
    let email = email.to_lowercase();

    let account = state
        .account_manager
        .get_account_by_email(
            &email,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        if let Some(email) = account.email {
            let token = state
                .account_manager
                .create_email_token(&account.did, EmailTokenPurpose::ResetPassword)
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            mailer::send_reset_password(
                email.clone(),
                IdentifierAndTokenParams {
                    identifier: account.handle.unwrap_or(email),
                    token,
                },
            )
            .await
            .map_err(|_| ApiError::RuntimeError)?;
            Ok(())
        } else {
            Err(ApiError::InvalidRequest(
                "Account does not have an email address".to_string(),
            ))
        }
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.requestPasswordReset
#[poem::handler]
pub async fn request_password_reset(
    body: Json<RequestPasswordResetInput>,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    match inner_request_password_reset(body.0, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
