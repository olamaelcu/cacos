use crate::account::helpers::account::AvailabilityFlags;
use crate::account::{AccountManager, EmailTokenPurpose, UpdateEmailOpts};
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::UpdateEmailInput;

async fn inner_update_email(
    body: UpdateEmailInput,
    auth: AccessFull,
    account_manager: &AccountManager,
) -> Result<(), ApiError> {
    let did = auth.access.credentials.unwrap().did.unwrap();
    let UpdateEmailInput { email, token } = body;
    // NOTE: rsky uses mailchecker::is_valid; cacos skips the third-party
    // mailchecker dependency and accepts any non-empty email.
    if email.is_empty() {
        return Err(ApiError::InvalidRequest(
            "This email address is not supported, please use a different email.".to_string(),
        ));
    }
    let account = account_manager
        .get_account(
            &did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: None,
            }),
        )
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        if account.email_confirmed_at.is_some() {
            if let Some(token) = token {
                account_manager
                    .assert_valid_email_token(&did, EmailTokenPurpose::UpdateEmail, &token)
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
            } else {
                return Err(ApiError::InvalidRequest(
                    "Confirmation token required".to_string(),
                ));
            }
        }
        account_manager
            .update_email(UpdateEmailOpts {
                did: did.clone(),
                email,
            })
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("UserAlreadyExistsError") {
                    ApiError::InvalidRequest(
                        "This email address is already in use, please use a different email."
                            .to_string(),
                    )
                } else {
                    ApiError::RuntimeError
                }
            })
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.updateEmail
#[poem::handler]
pub async fn update_email(
    body: Json<UpdateEmailInput>,
    auth: AccessFull,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    match inner_update_email(body.0, auth, &state.account_manager).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!("{error:?}");
            // Mirror rsky reference: any inner failure is surfaced as
            // RuntimeError so the wire shape is stable for clients.
            Err(ApiError::RuntimeError)
        }
    }
}