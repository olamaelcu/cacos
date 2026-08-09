use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.deleteSession — stub, filled in by a later task.
#[poem::handler]
pub async fn delete_session() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}