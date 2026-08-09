use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.requestEmailConfirmation — stub, filled in by a later task.
#[poem::handler]
pub async fn request_email_confirmation() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}