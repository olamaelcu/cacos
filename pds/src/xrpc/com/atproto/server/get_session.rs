use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// GET /xrpc/com.atproto.server.getSession — stub, filled in by a later task.
#[poem::handler]
pub async fn get_session() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}