use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.confirmEmail — stub, filled in by a later task.
#[poem::handler]
pub async fn confirm_email() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}