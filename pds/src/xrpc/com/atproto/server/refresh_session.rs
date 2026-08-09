use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// POST /xrpc/com.atproto.server.refreshSession — stub, filled in by a later task.
#[poem::handler]
pub async fn refresh_session() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}