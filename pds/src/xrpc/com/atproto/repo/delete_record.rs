//! `com.atproto.repo.deleteRecord` handler.

use crate::observability::timing::timed;
use crate::xrpc::com::atproto::repo::prepare::{PrepareDeleteOpts, prepare_delete};
use crate::xrpc::{ApiError, ApiResult, SharedState};
use lexicon_cid::Cid;
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::repo::DeleteRecordInput;
use rsky_repo::types::PreparedWrite;

fn requester_did(
    auth: &crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
) -> ApiResult<String> {
    auth.access
        .credentials
        .as_ref()
        .and_then(|c| c.did.clone())
        .ok_or_else(|| ApiError::InvalidRequest("Missing did on access token".to_string()))
}

async fn inner_delete_record(
    body: DeleteRecordInput,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<(), ApiError> {
    let did = requester_did(&auth)?;

    let swap_cid: Option<Cid> = body
        .swap_commit
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|e: lexicon_cid::Error| ApiError::InvalidRequest(format!("{e}")))?;

    let prepared = prepare_delete(PrepareDeleteOpts {
        did: did.clone(),
        collection: body.collection.clone(),
        rkey: body.rkey.clone(),
        swap_cid,
    })
    .map_err(|e| ApiError::InvalidRequest(format!("prepare_delete: {e}")))?;

    let write = PreparedWrite::Delete(prepared);

    let mut transactor = timed("repo_load", async {
        state
            .actor_store
            .transact(did.clone(), state.blobstore.clone())
            .await
    })
    .await
    .map_err(|_| ApiError::RuntimeError)?;

    let commit = timed("repo_write", async {
        transactor.process_writes(vec![write], swap_cid).await
    })
    .await
    .map_err(|_| ApiError::RuntimeError)?;

    let mut seq = state
        .sequencer
        .sequencer
        .read()
        .expect("sequencer lock poisoned")
        .clone();
    let _ = timed("seq_write", async {
        seq.sequence_commit(did.clone(), commit).await
    })
    .await
    .map_err(|_| ApiError::RuntimeError)?;

    Ok(())
}

/// POST /xrpc/com.atproto.repo.deleteRecord
#[poem::handler]
pub async fn delete_record(
    body: Json<DeleteRecordInput>,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: Data<&SharedState>,
) -> ApiResult<()> {
    inner_delete_record(body.0, auth, state.0).await
}
