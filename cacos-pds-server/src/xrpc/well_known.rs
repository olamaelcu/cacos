//! `/.well-known/atproto-did` handler.
//!
//! Maps the `Host` header to a local handle (validated against
//! `config.identity.service_handle_domains`) and answers with the account's
//! DID, or 404 `"User not found"`. The handler is a port of the git-pinned
//! `olamaelcu/rsky` fork at rev `aee5aec5ad9473d80232beab58ddba25a936298a`
//! (`rsky` crate's `src/well_known.rs`).

use crate::xrpc::{ApiResult, SharedState};
use poem::Request;
use poem::Response;
use poem::http::StatusCode;
use poem::web::Data;

/// `/.well-known/atproto-did`: maps the Host header to a handle and answers
/// with the account DID, or 404 "User not found".
#[poem::handler]
pub async fn well_known(state: Data<&SharedState>, req: &Request) -> ApiResult<Response> {
    let handle = match req.headers().get("host").and_then(|v| v.to_str().ok()) {
        Some(h) => h.to_string(),
        None => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .finish());
        }
    };
    let supported_handle = state
        .config
        .identity
        .service_handle_domains
        .iter()
        .any(|host| handle.ends_with(host.as_str()) || handle == host[1..]);
    if !supported_handle {
        return Ok(not_found());
    }
    match state.account_manager.get_account(&handle, None).await {
        Ok(Some(user)) => Ok(Response::builder()
            .status(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(user.did)),
        Ok(None) => Ok(not_found()),
        Err(_) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .content_type("text/plain; charset=utf-8")
            .body("Internal Server Error")),
    }
}

fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .content_type("text/plain; charset=utf-8")
        .body("User not found")
}
