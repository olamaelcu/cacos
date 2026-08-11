//! `com.atproto.repo.applyWrites` handler.
//!
//! Dispatches each [`ApplyWritesInputRefWrite`] action to the matching
//! `prepare_*` helper, then writes all of them in a single repo
//! transaction so `seq_write` coincides with one commit.

use cacos_pds_core::observability::timing::timed;
use crate::xrpc::com::atproto::repo::prepare::{
    PrepareCreateOpts, PrepareDeleteOpts, PrepareUpdateOpts, prepare_create, prepare_delete,
    prepare_update,
};
use crate::xrpc::{ApiError, ApiResult, SharedState};
use lexicon_cid::Cid;
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::repo::{
    ApplyWritesInput, ApplyWritesInputRefWrite, RefWriteCreate, RefWriteDelete, RefWriteUpdate,
};
use rsky_lexicon::com::atproto::space::ApplyWritesOutput;
use rsky_repo::types::{PreparedWrite, RepoRecord};
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

async fn dispatch_action(
    did: &str,
    action: ApplyWritesInputRefWrite,
    validate: bool,
) -> Result<PreparedWrite, ApiError> {
    match action {
        ApplyWritesInputRefWrite::Create(RefWriteCreate {
            collection,
            rkey,
            value,
        }) => {
            let record: RepoRecord = serde_json::from_value(value)
                .map_err(|e| ApiError::InvalidRequest(format!("create record: {e}")))?;
            let prepared = prepare_create(PrepareCreateOpts {
                did: did.to_owned(),
                collection,
                rkey,
                swap_cid: None,
                record,
                validate: Some(validate),
            })
            .await
            .map_err(|e| ApiError::InvalidRequest(format!("prepare_create: {e}")))?;
            Ok(PreparedWrite::Create(prepared))
        }
        ApplyWritesInputRefWrite::Update(RefWriteUpdate {
            collection,
            rkey,
            value,
        }) => {
            let record: RepoRecord = serde_json::from_value(value)
                .map_err(|e| ApiError::InvalidRequest(format!("update record: {e}")))?;
            let prepared = prepare_update(PrepareUpdateOpts {
                did: did.to_owned(),
                collection,
                rkey,
                swap_cid: None,
                record,
                validate: Some(validate),
            })
            .await
            .map_err(|e| ApiError::InvalidRequest(format!("prepare_update: {e}")))?;
            Ok(PreparedWrite::Update(prepared))
        }
        ApplyWritesInputRefWrite::Delete(RefWriteDelete { collection, rkey }) => {
            let prepared = prepare_delete(PrepareDeleteOpts {
                did: did.to_owned(),
                collection,
                rkey,
                swap_cid: None,
            })
            .map_err(|e| ApiError::InvalidRequest(format!("prepare_delete: {e}")))?;
            Ok(PreparedWrite::Delete(prepared))
        }
    }
}

async fn inner_apply_writes(
    body: ApplyWritesInput,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: &SharedState,
) -> Result<ApplyWritesOutput, ApiError> {
    let did = requester_did(&auth)?;
    let validate = body.validate.unwrap_or(true);

    let swap_cid: Option<Cid> = body
        .swap_commit
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|e: lexicon_cid::Error| ApiError::InvalidRequest(format!("{e}")))?;

    let mut prepared_writes: Vec<PreparedWrite> = Vec::with_capacity(body.writes.len());
    for action in body.writes {
        prepared_writes.push(dispatch_action(&did, action, validate).await?);
    }

    let mut transactor = timed("repo_load", async {
        state
            .actor_store
            .transact(did.clone(), state.blobstore.clone())
            .await
    })
    .await
    .map_err(|_| ApiError::RuntimeError)?;

    let commit = timed("repo_write", async {
        transactor.process_writes(prepared_writes, swap_cid).await
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

    Ok(ApplyWritesOutput {
        commit: None,
        results: None,
    })
}

/// POST /xrpc/com.atproto.repo.applyWrites
#[poem::handler]
pub async fn apply_writes(
    body: Json<ApplyWritesInput>,
    auth: crate::xrpc::auth_extractors::AccessStandardIncludeChecks,
    state: Data<&SharedState>,
) -> ApiResult<Json<ApplyWritesOutput>> {
    inner_apply_writes(body.0, auth, state.0).await.map(Json)
}
