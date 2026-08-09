use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.requestAccountDelete — stub, filled in by a later task.
#[poem::handler]
pub async fn request_account_delete() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}