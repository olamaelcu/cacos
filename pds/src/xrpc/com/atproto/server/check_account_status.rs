use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// GET /xrpc/com.atproto.server.checkAccountStatus — stub, filled in by a later task.
#[poem::handler]
pub async fn check_account_status() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}