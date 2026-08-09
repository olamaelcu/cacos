use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.requestEmailUpdate — stub, filled in by a later task.
#[poem::handler]
pub async fn request_email_update() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}