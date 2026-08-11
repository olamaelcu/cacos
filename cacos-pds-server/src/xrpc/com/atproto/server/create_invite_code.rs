use crate::xrpc::auth_extractors::RequireInviteAdmin;
use crate::xrpc::com::atproto::server::gen_invite_code;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::{
    AccountCodes, CreateInviteCodeInput, CreateInviteCodeOutput,
};

async fn inner_create_invite_code(
    body: CreateInviteCodeInput,
    state: &SharedState,
) -> Result<CreateInviteCodeOutput, ApiError> {
    let CreateInviteCodeInput {
        use_count,
        for_account,
    } = body;
    let code = gen_invite_code();

    match state
        .account_manager
        .create_invite_codes(
            vec![AccountCodes {
                codes: vec![code.clone()],
                account: for_account.unwrap_or("admin".to_owned()),
            }],
            use_count,
        )
        .await
    {
        Ok(_) => Ok(CreateInviteCodeOutput { code }),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}

/// POST /xrpc/com.atproto.server.createInviteCode
#[poem::handler]
pub async fn create_invite_code(
    body: Json<CreateInviteCodeInput>,
    _auth: RequireInviteAdmin,
    state: Data<&SharedState>,
) -> ApiResult<Json<CreateInviteCodeOutput>> {
    match inner_create_invite_code(body.0, state.0).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => Err(error),
    }
}
