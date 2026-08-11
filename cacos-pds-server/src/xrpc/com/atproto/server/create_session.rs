use crate::xrpc::{ApiError, ApiResult, SharedState};
use cacos_pds_account::account::helpers::account::AvailabilityFlags;
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::{CreateSessionInput, CreateSessionOutput};
use rsky_syntax::handle::INVALID_HANDLE;

fn api_error_from_account_error(e: anyhow::Error) -> ApiError {
    use cacos_pds_account::account::helpers::account::AccountHelperError;
    if let Some(AccountHelperError::AccountLocked) = e.downcast_ref::<AccountHelperError>() {
        ApiError::RateLimitExceeded
    } else {
        tracing::error!("{e:?}");
        ApiError::RuntimeError
    }
}

async fn inner_create_session(
    body: CreateSessionInput,
    state: &SharedState,
) -> Result<CreateSessionOutput, ApiError> {
    let CreateSessionInput {
        password,
        identifier,
    } = body;
    let identifier = identifier.to_lowercase();

    let flags = Some(AvailabilityFlags {
        include_deactivated: Some(true),
        include_taken_down: Some(true),
    });
    let user = if identifier.contains('@') {
        state
            .account_manager
            .get_account_by_email(&identifier, flags)
            .await
    } else {
        state.account_manager.get_account(&identifier, flags).await
    };
    if let Ok(Some(user)) = user {
        // Per-account lockout check before touching the password verifier.
        if state
            .account_manager
            .is_account_locked(&user.did)
            .await
            .map_err(api_error_from_account_error)?
        {
            return Err(ApiError::RateLimitExceeded);
        }
        let mut app_password_name: Option<String> = None;

        let valid_account_pass = match state
            .account_manager
            .verify_account_password(&user.did, &password)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                tracing::error!("{e:?}");
                return Err(ApiError::RuntimeError);
            }
        };
        if !valid_account_pass {
            match state
                .account_manager
                .verify_app_password(&user.did, &password)
                .await
            {
                Ok(res) => {
                    app_password_name = res;
                }
                Err(e) => {
                    tracing::error!("{e:?}");
                    return Err(ApiError::RuntimeError);
                }
            }
            if app_password_name.is_none() {
                // Record one failed-login regardless of how many verifiers ran.
                // The current attempt still surfaces as InvalidLogin (400);
                // the next attempt will hit the lockout check at the top of
                // this handler once the counter crosses the threshold.
                state
                    .account_manager
                    .record_failed_login_with_lockout(&user.did)
                    .await
                    .map_err(api_error_from_account_error)?;
                return Err(ApiError::InvalidLogin);
            }
        }
        // Successful credential check clears the failed-login counter.
        state
            .account_manager
            .clear_failed_logins(&user.did)
            .await
            .map_err(api_error_from_account_error)?;
        if user.takedown_ref.is_some() {
            return Err(ApiError::AccountTakendown);
        }
        let (access_jwt, refresh_jwt);
        match state
            .account_manager
            .create_session(user.did.clone(), app_password_name)
            .await
        {
            Ok(res) => {
                (access_jwt, refresh_jwt) = res;
            }
            Err(e) => return Err(api_error_from_account_error(e)),
        }
        Ok(CreateSessionOutput {
            did: user.did,
            did_doc: None,
            handle: user.handle.unwrap_or(INVALID_HANDLE.to_string()),
            email: user.email,
            email_confirmed: Some(user.email_confirmed_at.is_some()),
            access_jwt,
            refresh_jwt,
        })
    } else {
        Err(ApiError::InvalidLogin)
    }
}

/// POST /xrpc/com.atproto.server.createSession
#[poem::handler]
pub async fn create_session(
    body: Json<CreateSessionInput>,
    state: Data<&SharedState>,
) -> ApiResult<Json<CreateSessionOutput>> {
    match inner_create_session(body.0, state.0).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => Err(error),
    }
}
