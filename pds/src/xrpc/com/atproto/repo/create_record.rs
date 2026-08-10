//! `com.atproto.repo.createRecord` handler.
//!
//! Wraps the three repo-write boundaries (`repo_load`, `repo_write`,
//! `seq_write`) with `timed()` so the stage histogram and the
//! inter-event timing layer report them.

use crate::observability::timing::timed;
use crate::xrpc::com::atproto::repo::prepare::{
    PrepareCreateOpts, assert_valid_record, prepare_create, set_collection_name,
};
use crate::xrpc::{ApiError, ApiResult, SharedState};
use lexicon_cid::Cid;
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::repo::{CreateRecordInput, CreateRecordOutput};
use rsky_repo::types::{PreparedWrite, WriteOpAction};

fn requester_did(
    auth: &crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
) -> ApiResult<String> {
    auth.access
        .credentials
        .as_ref()
        .and_then(|c| c.did.clone())
        .ok_or_else(|| ApiError::InvalidRequest("Missing did on access token".to_string()))
}

async fn inner_create_record(
    body: CreateRecordInput,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<CreateRecordOutput, ApiError> {
    let did = requester_did(&auth)?;

    let validate = body.validate.unwrap_or(true);
    let record_json = body.record.clone();
    let record: rsky_repo::types::RepoRecord = serde_json::from_value(record_json)
        .map_err(|e| ApiError::InvalidRequest(format!("Record did not deserialize: {e}")))?;
    let record = set_collection_name(&body.collection, record, validate)
        .map_err(|e| ApiError::InvalidRequest(format!("{e}")))?;
    if validate {
        assert_valid_record(&record).map_err(|e| ApiError::InvalidRequest(format!("{e}")))?;
    }
    let swap_cid: Option<Cid> = body
        .swap_commit
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|e: lexicon_cid::Error| ApiError::InvalidRequest(format!("{e}")))?;

    let prepared = prepare_create(PrepareCreateOpts {
        did: did.clone(),
        collection: body.collection.clone(),
        rkey: body.rkey.clone(),
        swap_cid,
        record: record.clone(),
        validate: Some(validate),
    })
    .await
    .map_err(|e| ApiError::InvalidRequest(format!("prepare_create: {e}")))?;

    let write = match prepared.action {
        WriteOpAction::Create => PreparedWrite::Create(prepared.clone()),
        other => {
            return Err(ApiError::InvalidRequest(format!(
                "prepare_create returned non-create action: {other:?}"
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
        .expect("sequencer lock poisoned")
        .clone();
    let _ = timed("seq_write", async {
        seq.sequence_commit(did.clone(), commit).await
    })
    .await
    .map_err(|_| ApiError::RuntimeError)?;

    Ok(CreateRecordOutput {
        cid: prepared.cid.to_string(),
        uri: prepared.uri.clone(),
    })
}

/// POST /xrpc/com.atproto.repo.createRecord
#[poem::handler]
pub async fn create_record(
    body: Json<CreateRecordInput>,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: Data<&SharedState>,
) -> ApiResult<Json<CreateRecordOutput>> {
    inner_create_record(body.0, auth, state.0).await.map(Json)
}
