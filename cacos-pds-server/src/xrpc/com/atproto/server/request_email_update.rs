use cacos_pds_account::account::EmailTokenPurpose;
use cacos_pds_account::account::helpers::account::AvailabilityFlags;
use cacos_pds_mailer;
use cacos_pds_mailer::TokenParam;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::RequestEmailUpdateOutput;

async fn inner_request_email_update(
    auth: AccessFull,
    state: &SharedState,
) -> Result<RequestEmailUpdateOutput, ApiError> {
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
            let token_required = account.email_confirmed_at.is_some();
            if token_required {
                let token = state
                    .account_manager
                    .create_email_token(&did, EmailTokenPurpose::UpdateEmail)
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
                cacos_pds_mailer::send_update_email(email, TokenParam { token })
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
            }

            Ok(RequestEmailUpdateOutput { token_required })
        } else {
            Err(ApiError::InvalidRequest(
                "Account does not have an email address".to_string(),
            ))
        }
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

/// POST /xrpc/com.atproto.server.requestEmailUpdate
#[poem::handler]
pub async fn request_email_update(
    auth: AccessFull,
    state: Data<&SharedState>,
) -> ApiResult<Json<RequestEmailUpdateOutput>> {
    match inner_request_email_update(auth, &state).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
