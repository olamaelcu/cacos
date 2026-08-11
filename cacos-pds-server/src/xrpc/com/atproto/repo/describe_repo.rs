//! `com.atproto.repo.describeRepo` handler.

use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::Data;
use rsky_lexicon::com::atproto::repo::DescribeRepoOutput;
use serde_json::Value;

#[poem::handler]
pub async fn describe_repo(
    repo: poem::web::Query<DescribeRepoQuery>,
    state: Data<&SharedState>,
) -> ApiResult<Json<DescribeRepoOutput>> {
    let repo = repo.0.repo.clone();
    let account = state
        .account_manager
        .get_account(&repo, None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let user = account.ok_or(ApiError::AccountNotFound)?;

    let collections = state
        .actor_store
        .read(user.did.clone(), state.blobstore.clone())
        .await
        .map_err(|_| ApiError::RuntimeError)?
        .record
        .list_collections()
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    Ok(Json(DescribeRepoOutput {
        handle: user.handle.unwrap_or_else(|| "handle.invalid".to_string()),
        did: user.did,
        did_doc: Value::Null,
        collections,
        handle_is_correct: false,
    }))
}

#[derive(serde::Deserialize)]
pub struct DescribeRepoQuery {
    pub repo: String,
}

use poem::web::Json;
