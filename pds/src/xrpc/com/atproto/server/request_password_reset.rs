use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.requestPasswordReset — stub, filled in by a later task.
#[poem::handler]
pub async fn request_password_reset() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}