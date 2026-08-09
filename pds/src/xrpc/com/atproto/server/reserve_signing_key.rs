use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ReserveSigningKeyInput {
    pub did: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveSigningKeyOutput {
    pub signing_key: String,
}

/// POST /xrpc/com.atproto.server.reserveSigningKey
#[poem::handler]
pub async fn reserve_signing_key(
    body: Json<ReserveSigningKeyInput>,
    state: Data<&SharedState>,
) -> ApiResult<Json<ReserveSigningKeyOutput>> {
    let ReserveSigningKeyInput { did } = body.0;
    match state.actor_store.reserve_keypair(did.as_deref()).await {
        Ok(signing_key) => Ok(Json(ReserveSigningKeyOutput { signing_key })),
        Err(error) => {
            tracing::error!("{error}");
            Err(ApiError::RuntimeError)
        }
    }
}