use crate::account::helpers::auth::{create_service_jwt, ServiceJwtParams};
use crate::xrpc::auth_extractors::AccessFull;
use crate::xrpc::{ApiError, ApiResult};
use anyhow::{bail, Result};
use chrono::offset::Utc as UtcOffset;
use chrono::DateTime;
use poem::web::Json;
use rsky_common::time::{from_micros_to_utc, HOUR, MINUTE};
use rsky_lexicon::com::atproto::server::GetServiceAuthOutput;
use std::time::SystemTime;

/// Methods that may not be proxied or service-authed (pipethrough consts,
/// ported here because cacos defers the pipethrough module).
pub static PROTECTED_METHODS: &[&str] = &[
    "com.atproto.admin.sendEmail",
    "com.atproto.identity.requestPlcOperationSignature",
    "com.atproto.identity.signPlcOperation",
    "com.atproto.identity.updateHandle",
    "com.atproto.server.activateAccount",
    "com.atproto.server.confirmEmail",
    "com.atproto.server.createAppPassword",
    "com.atproto.server.deactivateAccount",
    "com.atproto.server.getAccountInviteCodes",
    "com.atproto.server.listAppPasswords",
    "com.atproto.server.requestAccountDelete",
    "com.atproto.server.requestEmailConfirmation",
    "com.atproto.server.requestEmailUpdate",
    "com.atproto.server.revokeAppPassword",
    "com.atproto.server.updateEmail",
];

pub static PRIVILEGED_METHODS: &[&str] = &[
    "chat.bsky.actor.deleteAccount",
    "chat.bsky.actor.exportAccountData",
    "chat.bsky.convo.deleteMessageForSelf",
    "chat.bsky.convo.getConvo",
    "chat.bsky.convo.getConvoForMembers",
    "chat.bsky.convo.getLog",
    "chat.bsky.convo.getMessages",
    "chat.bsky.convo.leaveConvo",
    "chat.bsky.convo.listConvos",
    "chat.bsky.convo.muteConvo",
    "chat.bsky.convo.sendMessage",
    "chat.bsky.convo.sendMessageBatch",
    "chat.bsky.convo.unmuteConvo",
    "chat.bsky.convo.updateRead",
    "com.atproto.server.createAccount",
];

pub async fn inner_get_service_auth(
    aud: String,
    exp: Option<u64>,
    lxm: Option<String>,
    auth: AccessFull,
) -> Result<String> {
    let credentials = auth.access.credentials.unwrap();
    let did = credentials.clone().did.unwrap();
    let exp = exp.map(|exp| exp * 1000);
    if let Some(exp) = exp {
        let system_time = SystemTime::now();
        let now: DateTime<UtcOffset> = system_time.into();
        let diff = from_micros_to_utc(exp as i64) - now;
        if diff.num_milliseconds() < 0 {
            bail!("BadExpiration: expiration is in past");
        } else if diff.num_milliseconds() > HOUR as i64 {
            bail!("BadExpiration: cannot request a token with an expiration more than an hour in the future");
        } else if lxm.is_none() && diff.num_milliseconds() > MINUTE as i64 {
            bail!("BadExpiration: cannot request a method-less token with an expiration more than a minute in the future");
        }
    }
    if let Some(ref lxm) = lxm {
        if PROTECTED_METHODS.contains(&lxm.as_str()) {
            bail!("cannot request a service auth token for the following protected method: {lxm}");
        }
        if credentials.is_privileged.unwrap_or(false) && PRIVILEGED_METHODS.contains(&lxm.as_str()) {
            bail!("insufficient access to request a service auth token for the following method: {lxm}");
        }
    }
    create_service_jwt(ServiceJwtParams {
        iss: did,
        aud,
        exp: None,
        lxm,
        jti: None,
    })
    .await
}

/// GET /xrpc/com.atproto.server.getServiceAuth?<aud>&<exp>&<lxm>
#[poem::handler]
pub async fn get_service_auth(
    poem::web::Query(query): poem::web::Query<GetServiceAuthQuery>,
    auth: AccessFull,
) -> ApiResult<Json<GetServiceAuthOutput>> {
    let GetServiceAuthQuery { aud, exp, lxm } = query;
    match inner_get_service_auth(aud, exp, lxm, auth).await {
        Ok(token) => Ok(Json(GetServiceAuthOutput { token })),
        Err(error) => {
            tracing::error!("Internal Error: {error}");
            Err(ApiError::RuntimeError)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GetServiceAuthQuery {
    pub aud: String,
    pub exp: Option<u64>,
    pub lxm: Option<String>,
}