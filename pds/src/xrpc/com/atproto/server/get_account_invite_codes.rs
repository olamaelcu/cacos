use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// GET /xrpc/com.atproto.server.getAccountInviteCodes — stub, filled in by a later task.
#[poem::handler]
pub async fn get_account_invite_codes() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}