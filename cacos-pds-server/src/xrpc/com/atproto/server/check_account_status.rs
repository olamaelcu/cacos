use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::com::atproto::server::is_valid_did_doc_for_service;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use futures::try_join;
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::CheckAccountStatusOutput;

async fn inner_check_account_status(
    auth: AccessFull,
    state: &SharedState,
) -> Result<CheckAccountStatusOutput, ApiError> {
    let requester = auth
        .access
        .credentials
        .ok_or(ApiError::InvalidRequest(
            "Missing credentials on access token".to_string(),
        ))?
        .did
        .ok_or(ApiError::InvalidRequest(
            "Missing did on access token".to_string(),
        ))?;

    let actor_store = state
        .actor_store
        .read(requester.clone(), state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let repo_root = {
        let storage_guard = actor_store.storage.read().await;
        storage_guard
            .get_root_detailed()
            .await
            .map_err(|_| ApiError::RuntimeError)?
    };
    let repo_blocks: i64 = {
        let storage_guard = actor_store.storage.read().await;
        storage_guard
            .count_blocks()
            .await
            .map_err(|_| ApiError::RuntimeError)? as i64
    };
    let (indexed_records, imported_blobs, expected_blobs) = try_join!(
        async {
            actor_store
                .record
                .record_count()
                .await
                .map(|n| n as i64)
                .map_err(|_| ApiError::RuntimeError)
        },
        async {
            actor_store
                .blob
                .blob_count()
                .await
                .map_err(|_| ApiError::RuntimeError)
        },
        async {
            actor_store
                .blob
                .record_blob_count()
                .await
                .map_err(|_| ApiError::RuntimeError)
        },
    )
    .map_err(|_: ApiError| ApiError::RuntimeError)?;

    let (activated, valid_did) = try_join!(
        async {
            state
                .account_manager
                .is_account_activated(&requester)
                .await
                .map_err(|_| ApiError::RuntimeError)
        },
        async {
            is_valid_did_doc_for_service(
                requester.clone(),
                state.plc_client.as_ref(),
                &state.config.service,
            )
            .await
            .map_err(|_| ApiError::RuntimeError)
        },
    )
    .map_err(|_: ApiError| ApiError::RuntimeError)?;

    Ok(CheckAccountStatusOutput {
        activated,
        valid_did,
        repo_commit: repo_root.cid.to_string(),
        repo_rev: repo_root.rev,
        repo_blocks,
        indexed_records,
        private_state_values: 0,
        expected_blobs,
        imported_blobs,
    })
}

/// GET /xrpc/com.atproto.server.checkAccountStatus
#[poem::handler]
pub async fn check_account_status(
    auth: AccessFull,
    state: Data<&SharedState>,
) -> ApiResult<Json<CheckAccountStatusOutput>> {
    match inner_check_account_status(auth, state.0).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("Internal Error: {error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
