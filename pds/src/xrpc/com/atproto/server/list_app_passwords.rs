use crate::xrpc::auth_extractors::AccessStandard;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::{AppPassword, ListAppPasswordsOutput};

/// GET /xrpc/com.atproto.server.listAppPasswords
#[poem::handler]
pub async fn list_app_passwords(
    auth: AccessStandard,
    state: Data<&SharedState>,
) -> ApiResult<Json<ListAppPasswordsOutput>> {
    let credentials = auth
        .access
        .credentials
        .ok_or(ApiError::InvalidRequest(
            "Missing credentials on access token".to_string(),
        ))?;
    let did = credentials
        .did
        .ok_or(ApiError::InvalidRequest("Missing did on access token".to_string()))?;
    match state.account_manager.list_app_passwords(&did).await {
        Ok(passwords) => {
            let passwords: Vec<AppPassword> = passwords
                .into_iter()
                .map(|(name, created_at)| AppPassword { name, created_at })
                .collect();
            Ok(Json(ListAppPasswordsOutput { passwords }))
        }
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}
