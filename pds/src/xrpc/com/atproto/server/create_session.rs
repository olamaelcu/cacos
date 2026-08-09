use crate::account::helpers::account::AvailabilityFlags;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::{CreateSessionInput, CreateSessionOutput};
use rsky_syntax::handle::INVALID_HANDLE;

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
                return Err(ApiError::InvalidLogin);
            }
        }
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
            Err(e) => {
                tracing::error!("{e:?}");
                return Err(ApiError::RuntimeError);
            }
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