//! `com.atproto.repo.putRecord` handler.

use cacos_pds_core::observability::timing::timed;
use crate::xrpc::com::atproto::repo::prepare::{
    PrepareUpdateOpts, prepare_update, set_collection_name,
};
use crate::xrpc::{ApiError, ApiResult, SharedState};
use lexicon_cid::Cid;
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::repo::{PutRecordInput, PutRecordOutput};
use rsky_repo::types::{PreparedWrite, WriteOpAction};
use tracing_unwrap::ResultExt;

fn requester_did(
    auth: &crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
) -> ApiResult<String> {
    auth.access
        .credentials
        .as_ref()
        .and_then(|c| c.did.clone())
        .ok_or_else(|| ApiError::InvalidRequest("Missing did on access token".to_string()))
}

async fn inner_put_record(
    body: PutRecordInput,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<PutRecordOutput, ApiError> {
    let did = requester_did(&auth)?;
    let validate = body.validate.unwrap_or(true);

    let record: rsky_repo::types::RepoRecord = serde_json::from_value(body.record.clone())
        .map_err(|e| ApiError::InvalidRequest(format!("Record did not deserialize: {e}")))?;
    let record = set_collection_name(&body.collection, record, validate)
        .map_err(|e| ApiError::InvalidRequest(format!("{e}")))?;

    let swap_cid: Option<Cid> = body
        .swap_commit
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|e: lexicon_cid::Error| ApiError::InvalidRequest(format!("{e}")))?;

    let prepared = prepare_update(PrepareUpdateOpts {
        did: did.clone(),
        collection: body.collection.clone(),
        rkey: body.rkey.clone(),
        swap_cid,
        record,
        validate: Some(validate),
    })
    .await
    .map_err(|e| ApiError::InvalidRequest(format!("prepare_update: {e}")))?;

    let write = match prepared.action {
        WriteOpAction::Update | WriteOpAction::Create => PreparedWrite::Update(prepared.clone()),
        other => {
            return Err(ApiError::InvalidRequest(format!(
                "prepare_update returned unexpected action: {other:?}"
            )));
        }
    };

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
        .expect_or_log("sequencer lock poisoned")
        .clone();
    let _ = timed("seq_write", async {
        seq.sequence_commit(did.clone(), commit).await
    })
    .await
    .map_err(|_| ApiError::RuntimeError)?;

    Ok(PutRecordOutput {
        cid: prepared.cid.to_string(),
        uri: prepared.uri.clone(),
    })
}

/// POST /xrpc/com.atproto.repo.putRecord
#[poem::handler]
pub async fn put_record(
    body: Json<PutRecordInput>,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: Data<&SharedState>,
) -> ApiResult<Json<PutRecordOutput>> {
    inner_put_record(body.0, auth, state.0).await.map(Json)
}
