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

    // Enumeration defense: always return Ok(()) and never leak whether the
    // account exists. When the account exists we schedule the mailer
    // fire-and-forget so the request returns immediately.
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

    if let Some(account) = account
        && let Some(account_email) = account.email
    {
        let manager = state.account_manager.clone();
        let did = account.did.clone();
        let identifier = account.handle.unwrap_or_else(|| account_email.clone());
        let account_email_clone = account_email.clone();
        tokio::spawn(async move {
            match manager
                .create_email_token(&did, EmailTokenPurpose::ResetPassword)
                .await
            {
                Ok(token) => {
                    if let Err(err) = mailer::send_reset_password(
                        account_email_clone,
                        IdentifierAndTokenParams { identifier, token },
                    )
                    .await
                    {
                        tracing::error!("mailer::send_reset_password failed: {err:?}");
                    }
                }
                Err(err) => {
                    tracing::error!("create_email_token(ResetPassword) failed for {did}: {err:?}");
                }
            }
        });
    }
    Ok(())
}

/// POST /xrpc/com.atproto.server.requestPasswordReset
#[poem::handler]
pub async fn request_password_reset(
    body: Json<RequestPasswordResetInput>,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    match inner_request_password_reset(body.0, state.0).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
