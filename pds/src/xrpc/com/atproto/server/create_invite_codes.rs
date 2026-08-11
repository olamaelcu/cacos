use crate::xrpc::auth_extractors::RequireInviteAdmin;
use crate::xrpc::com::atproto::server::gen_invite_codes;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use poem::web::{Data, Json};
use rsky_lexicon::com::atproto::server::{
    AccountCodes, CreateInviteCodesInput, CreateInviteCodesOutput,
};

async fn inner_create_invite_codes(
    body: CreateInviteCodesInput,
    state: &SharedState,
) -> Result<CreateInviteCodesOutput, ApiError> {
    let CreateInviteCodesInput {
        use_count,
        code_count,
        for_accounts,
    } = body;
    let for_accounts = for_accounts.unwrap_or_else(|| vec!["admin".to_owned()]);

    let mut account_codes: Vec<AccountCodes> = Vec::new();
    for account in for_accounts {
        let codes = gen_invite_codes(code_count);
        account_codes.push(AccountCodes { account, codes });
    }

    match state
        .account_manager
        .create_invite_codes(account_codes.clone(), use_count)
        .await
    {
        Ok(_) => Ok(CreateInviteCodesOutput {
            codes: account_codes,
        }),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}

/// POST /xrpc/com.atproto.server.createInviteCodes
#[poem::handler]
pub async fn create_invite_codes(
    body: Json<CreateInviteCodesInput>,
    _auth: RequireInviteAdmin,
    state: Data<&SharedState>,
) -> ApiResult<Json<CreateInviteCodesOutput>> {
    match inner_create_invite_codes(body.0, state.0).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => Err(error),
    }
}
