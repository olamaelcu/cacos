use cacos_pds_account::account::helpers::invite::CodeDetail;
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::com::atproto::server::gen_invite_codes;
use crate::xrpc::{ApiError, ApiResult, SharedState};
use chrono::NaiveDateTime;
use poem::web::{Data, Json};
use rsky_common::RFC3339_VARIANT;
use rsky_common::env::{env_bool, env_int};
use rsky_lexicon::com::atproto::server::GetAccountInviteCodesOutput;
use std::time::SystemTime;

struct CalculateCodesToCreateOpts {
    pub user_created_at: usize,
    pub codes: Vec<CodeDetail>,
    pub epoch: usize,
    pub interval: usize,
}

fn calculate_codes_to_create(opts: CalculateCodesToCreateOpts) -> Result<(usize, usize), ApiError> {
    let routine_codes: Vec<CodeDetail> = opts
        .codes
        .into_iter()
        .filter(|code| code.created_by != "admin")
        .collect();
    let unused_routine_codes: Vec<CodeDetail> = routine_codes
        .clone()
        .into_iter()
        .filter(|row| !row.disabled && row.available as usize > row.uses.len())
        .collect();

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("timestamp in micros since UNIX epoch")
        .as_micros() as usize;
    let user_lifespan = now - opts.user_created_at;

    let could_create: usize = if opts.user_created_at >= opts.epoch {
        user_lifespan / opts.interval
    } else {
        let could_create_total = user_lifespan / opts.interval;
        let user_pre_epoch_lifespan = opts.epoch - opts.user_created_at;
        let could_create_before_epoch = user_pre_epoch_lifespan / opts.interval;
        could_create_total - could_create_before_epoch
    };
    let epoch_codes: Vec<CodeDetail> = routine_codes
        .clone()
        .into_iter()
        .filter(|code| {
            let datetime = NaiveDateTime::parse_from_str(&code.created_at, RFC3339_VARIANT)
                .unwrap()
                .and_utc()
                .timestamp_micros() as usize;
            datetime > opts.epoch
        })
        .collect();
    let to_create = std::cmp::min(
        5usize.saturating_sub(unused_routine_codes.len()),
        could_create.saturating_sub(epoch_codes.len()),
    );
    Ok((to_create, routine_codes.len() + to_create))
}

async fn inner_get_account_invite_codes(
    include_used: bool,
    create_available: bool,
    auth: AccessFull,
    state: &SharedState,
) -> Result<GetAccountInviteCodesOutput, ApiError> {
    let requester = auth.access.credentials.unwrap().did.unwrap();
    let account = state
        .account_manager
        .get_account(&requester, None)
        .await
        .map_err(|_| ApiError::RuntimeError)?;
    let mut user_codes = state
        .account_manager
        .get_account_invite_codes(&requester)
        .await
        .map_err(|_| ApiError::RuntimeError)?;

    if let Some(account) = account {
        let mut created: Vec<CodeDetail> = Vec::new();
        if create_available
            && env_bool("PDS_INVITE_REQUIRED").unwrap_or(true)
            && env_int("PDS_INVITE_INTERVAL").is_some()
        {
            let user_created_at =
                NaiveDateTime::parse_from_str(&account.created_at, RFC3339_VARIANT)
                    .map_err(|_| ApiError::RuntimeError)?
                    .and_utc()
                    .timestamp_micros() as usize;
            let (to_create, total) = calculate_codes_to_create(CalculateCodesToCreateOpts {
                user_created_at,
                codes: user_codes.clone(),
                epoch: env_int("PDS_INVITE_EPOCH").unwrap_or(0),
                interval: env_int("PDS_INVITE_INTERVAL").unwrap(),
            })?;
            if to_create > 0 {
                let codes = gen_invite_codes(to_create as i32);
                created = state
                    .account_manager
                    .create_account_invite_codes(
                        &requester,
                        codes,
                        total,
                        account.invites_disabled.unwrap_or(0) == 1,
                    )
                    .await
                    .map_err(|_| ApiError::RuntimeError)?;
            }
        }
        let mut all_codes: Vec<CodeDetail> = Vec::new();
        all_codes.append(&mut created);
        all_codes.append(&mut user_codes);
        let filtered: Vec<CodeDetail> = all_codes
            .into_iter()
            .filter(|code| {
                if code.disabled {
                    return false;
                }
                if !include_used && code.uses.len() >= code.available as usize {
                    return false;
                }
                true
            })
            .collect();
        Ok(GetAccountInviteCodesOutput { codes: filtered })
    } else {
        Err(ApiError::InvalidRequest("Account not found".to_string()))
    }
}

#[derive(serde::Deserialize)]
pub struct GetAccountInviteCodesQuery {
    #[serde(rename = "includeUsed")]
    pub include_used: bool,
    #[serde(rename = "createAvailable")]
    pub create_available: bool,
}

/// GET /xrpc/com.atproto.server.getAccountInviteCodes?<includeUsed>&<createAvailable>
#[poem::handler]
pub async fn get_account_invite_codes(
    poem::web::Query(query): poem::web::Query<GetAccountInviteCodesQuery>,
    auth: AccessFull,
    state: Data<&SharedState>,
) -> ApiResult<Json<GetAccountInviteCodesOutput>> {
    let GetAccountInviteCodesQuery {
        include_used,
        create_available,
    } = query;
    match inner_get_account_invite_codes(include_used, create_available, auth, state.0).await {
        Ok(res) => Ok(Json(res)),
        Err(error) => {
            tracing::error!("{error:?}");
            Err(ApiError::RuntimeError)
        }
    }
}
