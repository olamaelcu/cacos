use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::{CreateAppPasswordInput, CreateAppPasswordOutput};

/// POST /xrpc/com.atproto.server.createAppPassword
#[poem::handler]
pub async fn create_app_password(
    body: Json<CreateAppPasswordInput>,
    auth: AccessStandard,
    state: Data<&SharedState>,
) -> ApiResult<Json<CreateAppPasswordOutput>> {
    let CreateAppPasswordInput { name } = body.0;
    let credentials = auth
        .access
        .credentials
        .ok_or(ApiError::InvalidRequest(
            "Missing credentials on access token".to_string(),
        ))?;
    let did = credentials
        .did
        .ok_or(ApiError::InvalidRequest("Missing did on access token".to_string()))?;
    match state.account_manager.create_app_password(did, name).await {
        Ok(app_password) => Ok(Json(app_password)),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
