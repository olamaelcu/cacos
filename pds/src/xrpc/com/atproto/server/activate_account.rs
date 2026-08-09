use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.activateAccount — stub, filled in by a later task.
#[poem::handler]
pub async fn activate_account() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}