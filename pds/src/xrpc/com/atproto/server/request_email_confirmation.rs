use crate::account::helpers::account::AvailabilityFlags;
use crate::account::EmailTokenPurpose;
use crate::mailer;
use crate::mailer::TokenParam;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Data;

async fn inner_request_email_confirmation(
    auth: AccessFull,
    state: &SharedState,
) -> Result<(), ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();
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
    if let Some(account) = account {
        if let Some(email) = account.email {
            let token = state
                .account_manager
                .create_email_token(&did, EmailTokenPurpose::ConfirmEmail)
                .await
                .map_err(|_| ApiError::RuntimeError)?;
            mailer::send_confirm_email(email, TokenParam { token })
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

/// POST /xrpc/com.atproto.server.requestEmailConfirmation
#[poem::handler]
pub async fn request_email_confirmation(
    auth: AccessFull,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    match inner_request_email_confirmation(auth, &state).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}