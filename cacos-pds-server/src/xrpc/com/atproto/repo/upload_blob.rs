//! `com.atproto.repo.uploadBlob` handler.
//!
//! Stores the request body bytes as a permanent blob and tags the
//! `blob_put` write with `timed()` so the stage histogram reflects
//! upload duration.

use crate::xrpc::auth_extractors::AccessStandardIncludeChecks;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use cacos_pds_core::observability::timing::timed;
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::repo::{Blob, BlobOutput};
use sha2::{Digest, Sha256};

fn requester_did(auth: &AccessStandardIncludeChecks) -> ApiResult<String> {
    auth.access
        .credentials
        .as_ref()
        .and_then(|c| c.did.clone())
        .ok_or_else(|| ApiError::InvalidRequest("Missing did on access token".to_string()))
}

/// POST /xrpc/com.atproto.repo.uploadBlob
#[poem::handler]
pub async fn upload_blob(
    auth: AccessStandardIncludeChecks,
    body: poem::Body,
    state: Data<&SharedState>,
) -> ApiResult<Json<BlobOutput>> {
    let _did = requester_did(&auth)?;
    let bytes: Vec<u8> = body.into_vec().await.map_err(|_| ApiError::RuntimeError)?;
    let cid = rsky_common::ipld::sha256_to_cid(Sha256::digest(&bytes).to_vec());

    let cid = timed("blob_put", async {
        state
            .blobstore
            .put_permanent(cid, bytes.clone())
            .await
            .map_err(|_| ApiError::RuntimeError)?;
        Ok::<_, ApiError>(cid)
    })
    .await?;

    Ok(Json(BlobOutput {
        blob: Blob {
            r#type: Some("blob".to_string()),
            r#ref: Some(cid),
            cid: Some(cid.to_string()),
            mime_type: "application/octet-stream".to_string(),
            size: Some(bytes.len() as i64),
            original: None,
        },
    }))
}
