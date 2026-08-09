use crate::xrpc::{ApiError, ApiResult};
use poem::web::Json;

/// GET /xrpc/com.atproto.server.listAppPasswords — stub, filled in by a later task.
#[poem::handler]
pub async fn list_app_passwords() -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::RuntimeError)
}