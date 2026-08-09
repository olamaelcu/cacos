use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.updateEmail — stub, filled in by a later task.
#[poem::handler]
pub async fn update_email() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}