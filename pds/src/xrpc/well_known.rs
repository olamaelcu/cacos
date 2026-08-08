//! `/.well-known/atproto-did` handler.
//!
//! Returns the service DID so clients can resolve the hosting PDS account
//! without an extra round trip. The DID is read from `PDS_SERVICE_DID`,
//! falling back to a `did:web:<hostname>` derivation when unset.

use crate::xrpc::{ApiResult, SharedState};
use poem::http::StatusCode;
use poem::web::Data;
use poem::Response;

/// `/.well-known/atproto-did`: echoes the service DID as a text/plain body.
#[poem::handler]
pub async fn well_known(state: Data<&SharedState>) -> ApiResult<Response> {
    let did = std::env::var("PDS_SERVICE_DID")
        .unwrap_or_else(|_| format!("did:web:{}", state.config.service.hostname));
    Ok(Response::builder()
        .status(StatusCode::OK)
        .content_type("text/plain; charset=utf-8")
        .body(did))
}
