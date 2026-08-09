use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.resetPassword — stub, filled in by a later task.
#[poem::handler]
pub async fn reset_password() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}