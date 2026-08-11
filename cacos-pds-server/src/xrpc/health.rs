//! Health endpoints: `/xrpc/_health` and `/xrpc/_health/live`.
//!
//! `health` touches the account DB and reports 503 on failure. `health_live`
//! is a pure liveness probe that never touches the DB (suitable for
//! Kubernetes `livenessProbe`).

use crate::xrpc::{ApiResult, SharedState};
use poem::Response;
use poem::http::StatusCode;
use poem::web::Data;
use sea_orm::ConnectionTrait;
use serde::Serialize;

#[derive(Serialize)]
pub struct ServerVersion {
    pub version: String,
}

/// `/xrpc/_health`: touches the account DB; 503 with a JSON body on
/// failure.
#[poem::handler]
pub async fn health(state: Data<&SharedState>) -> ApiResult<Response> {
    let ok = state
        .account_manager
        .db
        .execute_unprepared("SELECT 1")
        .await;
    match ok {
        Ok(_) => {
            let version = std::env::var("PDS_VERSION").unwrap_or_else(|_| "0.0.0-test".into());
            Ok(Response::builder()
                .status(StatusCode::OK)
                .content_type("application/json")
                .body(serde_json::to_vec(&ServerVersion { version }).unwrap()))
        }
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Ok(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .content_type("application/json")
                .body(
                    serde_json::json!({
                        "error": "ServiceUnavailable",
                        "message": error.to_string(),
                    })
                    .to_string(),
                ))
        }
    }
}

/// `/xrpc/_health/live`: pure liveness probe, never touches the DB.
#[poem::handler]
pub async fn health_live() -> &'static str {
    "ok"
}
