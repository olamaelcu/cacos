//! HTTP bearer / basic auth header parsing + JWT-verify orchestration.
//!
//! The DPoP-bound access-token branch is also implemented here — the
//! DPoP request itself is delegated to `rsky_oauth::OAuthProvider`, but
//! the entry point that chooses between bearer and DPoP based on the
//! `Authorization` header (`validate_access_token`) lives in this
//! module.

use super::admin::{
    check_account_status, AccountStatus,
};
use super::register::{current_dpop_request_context, ACCOUNT_MANAGER, OAUTH_PROVIDER, SIGNING_KEY_RESOLVER};
use super::service_jwt::{service_did, verify_service_jwt, ServiceJwtOpts};

use crate::account::helpers::account::{ActorAccount, AvailabilityFlags};
use crate::account::helpers::admin_tokens::AdminScopeSet;
use crate::account::helpers::auth::{AuthScope, CustomClaimObj};
use crate::auth::PDS_JWT_KEYPAIR;

use anyhow::{Result, bail};
use base64ct::{Base64, Encoding as _};
use jwt_simple::prelude::*;
use rsky_oauth::dpop::DpopRequest;
use rsky_oauth::VerifiedAccess;
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const INFINITY: u64 = u64::MAX;

const BEARER: &str = "Bearer ";
const BASIC: &str = "Basic ";
const DPOP: &str = "DPoP ";

#[derive(Clone, Debug)]
pub struct Credentials {
    pub r#type: String,
    pub did: Option<String>,
    pub scope: Option<AuthScope>,
    pub audience: Option<String>,
    pub token_id: Option<String>,
    pub aud: Option<String>,
    pub iss: Option<String>,
    pub is_privileged: Option<bool>,
    /// Granted admin scopes when this credential came from an admin token
    /// (basic-auth). `None` for access / refresh / user_did credentials and
    /// for admin credentials where the registry returned no entry (the
    /// extractors that need admin scope fail closed when this is `None`).
    pub admin_scopes: Option<AdminScopeSet>,
}

#[derive(Clone, Debug)]
pub struct AccessOutput {
    pub credentials: Option<Credentials>,
    pub artifacts: Option<String>,
}

#[derive(Debug)]
pub struct ValidatedBearer {
    pub did: String,
    pub scope: AuthScope,
    pub token: String,
    pub payload: JwtPayload,
    pub audience: Option<String>,
}

pub struct ValidateAccessTokenOpts {
    pub check_takedown: Option<bool>,
    pub check_deactivated: Option<bool>,
}

pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug)]
pub struct JwtPayload {
    pub scope: AuthScope,
    pub sub: Option<String>,
    pub aud: Option<Audiences>,
    pub exp: Option<Duration>,
    pub iat: Option<Duration>,
    pub jti: Option<String>,
}

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("ExpiredToken: `Token is expired`")]
    ExpiredToken,
    #[error("BadJwt: `{0}`")]
    BadJwt(String),
    #[error("BadJwtAudience: `{0}`")]
    BadJwtAudience(String),
    #[error("UntrustedIss: `{0}`")]
    UntrustedIss(String),
    #[error("AuthRequired: `{0}`")]
    AuthRequired(String),
    #[error("AccountNotFound: `{0}`")]
    AccountNotFound(String),
    #[error("AccountTakedown: `{0}`")]
    AccountTakedown(String),
    #[error("AccountDeactivated: `{0}`")]
    AccountDeactivated(String),
    #[error("InternalServerError: `{0}`")]
    InternalServerError(String),
}

#[derive(Debug, Default, Clone)]
pub struct OAuthResponseHeaders {
    pub dpop_nonce: Option<String>,
    pub www_authenticate: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DpopRequestInput<'a> {
    pub method: &'a str,
    pub uri: &'a str,
    pub dpop_headers: Vec<&'a str>,
}

impl DpopRequestInput<'_> {
    pub fn dpop_request<'b>(&'b self, access_token: Option<&'b str>) -> DpopRequest<'b> {
        DpopRequest {
            method: self.method,
            uri: self.uri,
            dpop_headers: &self.dpop_headers,
            access_token,
        }
    }
}

pub fn verify_jwt(jwt: &str, verify_options: Option<VerificationOptions>) -> Result<JwtPayload> {
    let claims = PDS_JWT_KEYPAIR
        .public_key()
        .verify_token::<CustomClaimObj>(jwt, verify_options)?;
    Ok(JwtPayload {
        scope: AuthScope::from_str(&claims.custom.scope)?,
        sub: claims.subject,
        aud: claims.audiences,
        exp: claims.expires_at,
        iat: claims.issued_at,
        jti: claims.jwt_id,
    })
}

/// Maps the granted OAuth scopes onto the closest legacy [`AuthScope`],
/// mirroring the upstream transition-scope semantics.
pub fn oauth_scopes_to_auth_scope(scopes: &[String]) -> Result<AuthScope> {
    let has = |scope: &str| scopes.iter().any(|granted| granted == scope);
    if !has("atproto") {
        bail!("Bad token scope")
    }
    if has("transition:chat.bsky") {
        Ok(AuthScope::AppPassPrivileged)
    } else if has("transition:generic") {
        Ok(AuthScope::AppPass)
    } else {
        bail!("Bad token scope")
    }
}

pub fn is_bearer_token(auth_header: Option<&str>) -> bool {
    match auth_header {
        None => false,
        Some(header) => header.starts_with(BEARER),
    }
}

pub fn is_basic_token(auth_header: Option<&str>) -> bool {
    match auth_header {
        None => false,
        Some(header) => header.starts_with(BASIC),
    }
}

pub fn bearer_token_from_req(auth_header: Option<&str>) -> Result<Option<String>> {
    match auth_header {
        Some(header) if !header.starts_with(BEARER) => Ok(None),
        Some(header) => {
            let slice = &header[BEARER.len()..];
            Ok(Some(slice.to_string()))
        }
        None => Ok(None),
    }
}

pub fn dpop_token_from_req(auth_header: Option<&str>) -> Option<String> {
    match auth_header {
        Some(header)
            if header.len() > DPOP.len() && header[..DPOP.len()].eq_ignore_ascii_case(DPOP) =>
        {
            Some(header[DPOP.len()..].to_string())
        }
        _ => None,
    }
}

/// Returns the current unix timestamp in seconds (UTC). Used by both
/// `validate_dpop_access_token` and `verify_service_jwt_token`.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

/// True when `err` is jwt-simple's expiry error (`JWTError::TokenHasExpired`),
/// as opposed to a signature, format, or other verification failure.
pub(crate) fn is_expired_jwt(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<jwt_simple::JWTError>(),
        Some(jwt_simple::JWTError::TokenHasExpired)
    )
}

pub fn parse_basic_auth(token: &str) -> Option<BasicAuth> {
    let mut parts = token.split_whitespace();
    if parts.next() != Some("Basic") {
        return None;
    }
    let b64 = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let decoded: Vec<u8> = match Base64::decode_vec(b64) {
        Err(_) => return None,
        Ok(decoded) => decoded,
    };
    let parsed_str: &str = match str::from_utf8(&decoded) {
        Err(_) => return None,
        Ok(res) => res,
    };
    let (username, password) = parsed_str.split_once(':')?;
    Some(BasicAuth {
        username: username.to_string(),
        password: password.to_string(),
    })
}

/// Pure bearer-token validation (no Request, no DB).
pub fn validate_bearer_token(
    token: &str,
    scopes: Vec<AuthScope>,
    verify_options: Option<VerificationOptions>,
) -> Result<ValidatedBearer> {
    let payload = verify_jwt(token, verify_options)?;
    let JwtPayload {
        sub, aud, scope, ..
    } = payload.clone();
    let sub = sub.unwrap();
    let aud = aud.unwrap();
    if !sub.starts_with("did:") {
        bail!("Malformed token")
    }
    if let Audiences::AsString(aud) = aud {
        if !aud.starts_with("did:") {
            bail!("Malformed token")
        }
        if !scopes.is_empty() && !scopes.contains(&scope) {
            bail!("Bad token scope")
        }
        Ok(ValidatedBearer {
            did: sub,
            scope,
            audience: Some(aud),
            token: token.to_string(),
            payload,
        })
    } else {
        bail!("Malformed token")
    }
}

pub fn validate_bearer_access_token(
    auth_header: Option<&str>,
    scopes: Vec<AuthScope>,
    service_did: &str,
) -> Result<AccessOutput> {
    let options = VerificationOptions {
        allowed_audiences: Some(HashSet::from_strings(&[service_did.to_string()])),
        ..Default::default()
    };
    let ValidatedBearer {
        did,
        scope,
        token,
        audience,
        ..
    } = validate_bearer_token(
        &bearer_token_from_req(auth_header)?.unwrap_or_default(),
        scopes,
        Some(options),
    )?;
    let is_privileged = [AuthScope::Access, AuthScope::AppPassPrivileged].contains(&scope);
    Ok(AccessOutput {
        credentials: Some(Credentials {
            r#type: "access".to_string(),
            did: Some(did),
            scope: Some(scope),
            audience,
            token_id: None,
            aud: None,
            iss: None,
            is_privileged: Some(is_privileged),
            admin_scopes: None,
        }),
        artifacts: Some(token),
    })
}

/// Fetches the account projection for [`check_account_status`] from the
/// registered `AccountManager`.
pub async fn fetch_account_status(did: &str) -> Result<Option<AccountStatus>, AuthError> {
    let account_manager = ACCOUNT_MANAGER
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(account_manager) = account_manager else {
        return Err(AuthError::InternalServerError(
            "AccountManager is not registered".to_string(),
        ));
    };
    let found: ActorAccount = match account_manager
        .get_account(
            did,
            Some(AvailabilityFlags {
                include_deactivated: Some(true),
                include_taken_down: Some(true),
            }),
        )
        .await
    {
        Ok(Some(found)) => found,
        _ => {
            return Err(AuthError::AccountNotFound("Account not found".to_string()));
        }
    };
    Ok(Some(AccountStatus {
        deactivated_at: found.deactivated_at,
        takedown_ref: found.takedown_ref,
    }))
}

/// Validates a refresh token.
pub async fn validate_refresh_token(
    auth_header: Option<&str>,
) -> Result<ValidatedBearer, AuthError> {
    let options = VerificationOptions {
        allowed_audiences: Some(HashSet::from_strings(&[service_did()])),
        max_validity: Some(Duration::from_secs(INFINITY)),
        ..Default::default()
    };
    let Some(token) = bearer_token_from_req(auth_header).ok().flatten() else {
        return Err(AuthError::AuthRequired("AuthMissing".to_string()));
    };
    match validate_bearer_token(&token, vec![AuthScope::Refresh], Some(options)) {
        Ok(bearer) => {
            metrics::counter!("cacos_auth_requests_total", "kind" => "refresh").increment(1);
            Ok(bearer)
        }
        Err(error) => {
            metrics::counter!("cacos_auth_failures_total", "kind" => "refresh").increment(1);
            Err(if is_expired_jwt(&error) {
                AuthError::ExpiredToken
            } else {
                AuthError::BadJwt(error.to_string())
            })
        }
    }
}

/// Validates a DPoP-bound OAuth access token against the registered
/// provider, mapping the granted scopes onto the legacy [`AuthScope`]
/// model.
pub async fn validate_dpop_access_token(
    token: String,
    scopes: Vec<AuthScope>,
    opts: Option<ValidateAccessTokenOpts>,
) -> Result<AccessOutput, AuthError> {
    let provider = OAUTH_PROVIDER
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(provider) = provider else {
        return Err(AuthError::InternalServerError(
            "OAuth provider is not configured".to_string(),
        ));
    };
    let Some(ctx) = current_dpop_request_context() else {
        return Err(AuthError::BadJwt(
            "DPoP-bound access tokens require request context (set_dpop_request_context)"
                .to_string(),
        ));
    };
    let now = now_secs();
    let dpop_headers: Vec<&str> = ctx.dpop_headers.iter().map(String::as_str).collect();
    let dpop_input = DpopRequestInput {
        method: &ctx.method,
        uri: &ctx.uri,
        dpop_headers,
    };
    let dpop = dpop_input.dpop_request(Some(&token));
    let verified: VerifiedAccess = provider
        .verify_access_token(&token, &dpop, now)
        .await
        .map_err(|error| {
            metrics::counter!("cacos_auth_failures_total", "kind" => "oauth").increment(1);
            AuthError::BadJwt(error.error_description().to_string())
        })?;
    let scope = oauth_scopes_to_auth_scope(&verified.scopes).map_err(|error| {
        metrics::counter!("cacos_auth_failures_total", "kind" => "oauth").increment(1);
        AuthError::BadJwt(error.to_string())
    })?;
    if !scopes.is_empty() && !scopes.contains(&scope) {
        metrics::counter!("cacos_auth_failures_total", "kind" => "oauth").increment(1);
        return Err(AuthError::BadJwt("Bad token scope".to_string()));
    }
    let ValidateAccessTokenOpts {
        check_takedown,
        check_deactivated,
    } = opts.unwrap_or(ValidateAccessTokenOpts {
        check_takedown: Some(false),
        check_deactivated: Some(false),
    });
    let account_status = if check_takedown.unwrap_or(false) || check_deactivated.unwrap_or(false) {
        fetch_account_status(&verified.did).await?
    } else {
        None
    };
    if let Err(error) = check_account_status(
        account_status.as_ref(),
        check_takedown.unwrap_or(false),
        check_deactivated.unwrap_or(false),
    ) {
        metrics::counter!("cacos_auth_failures_total", "kind" => "oauth").increment(1);
        return Err(error);
    }
    metrics::counter!("cacos_auth_requests_total", "kind" => "oauth").increment(1);
    Ok(AccessOutput {
        credentials: Some(Credentials {
            r#type: "oauth".to_string(),
            did: Some(verified.did),
            scope: Some(scope),
            audience: Some(service_did()),
            token_id: Some(verified.token_id),
            aud: None,
            iss: None,
            is_privileged: None,
            admin_scopes: None,
        }),
        artifacts: Some(token),
    })
}

/// Validates an access token presented to the resource server. Routes into
/// the DPoP branch when the `Authorization` header carries the `DPoP`
/// scheme, otherwise verifies a legacy bearer JWT against `PDS_SERVICE_DID`.
pub async fn validate_access_token(
    auth_header: Option<&str>,
    scopes: Vec<AuthScope>,
    opts: Option<ValidateAccessTokenOpts>,
) -> Result<AccessOutput, AuthError> {
    cacos_pds_core::observability::timing::timed("auth", async {
        validate_access_token_inner(auth_header, scopes, opts).await
    })
    .await
}

async fn validate_access_token_inner(
    auth_header: Option<&str>,
    scopes: Vec<AuthScope>,
    opts: Option<ValidateAccessTokenOpts>,
) -> Result<AccessOutput, AuthError> {
    if let Some(token) = dpop_token_from_req(auth_header) {
        return validate_dpop_access_token(token, scopes, opts).await;
    }
    let options = VerificationOptions {
        allowed_audiences: Some(HashSet::from_strings(&[service_did()])),
        ..Default::default()
    };
    let Some(token) = bearer_token_from_req(auth_header).ok().flatten() else {
        metrics::counter!("cacos_auth_failures_total", "kind" => "access").increment(1);
        return Err(AuthError::AuthRequired("AuthMissing".to_string()));
    };
    let ValidatedBearer {
        did,
        scope,
        token,
        audience,
        ..
    } = match validate_bearer_token(&token, scopes, Some(options)) {
        Ok(validated) => validated,
        Err(error) => {
            metrics::counter!("cacos_auth_failures_total", "kind" => "access").increment(1);
            return Err(if is_expired_jwt(&error) {
                AuthError::ExpiredToken
            } else {
                AuthError::BadJwt(error.to_string())
            });
        }
    };
    let ValidateAccessTokenOpts {
        check_takedown,
        check_deactivated,
    } = opts.unwrap_or(ValidateAccessTokenOpts {
        check_takedown: Some(false),
        check_deactivated: Some(false),
    });
    let account_status = if check_takedown.unwrap_or(false) || check_deactivated.unwrap_or(false) {
        fetch_account_status(&did).await?
    } else {
        None
    };
    if let Err(error) = check_account_status(
        account_status.as_ref(),
        check_takedown.unwrap_or(false),
        check_deactivated.unwrap_or(false),
    ) {
        metrics::counter!("cacos_auth_failures_total", "kind" => "access").increment(1);
        return Err(error);
    }
    metrics::counter!("cacos_auth_requests_total", "kind" => "access").increment(1);
    Ok(AccessOutput {
        credentials: Some(Credentials {
            r#type: "access".to_string(),
            did: Some(did),
            scope: Some(scope),
            audience,
            token_id: None,
            aud: None,
            iss: None,
            is_privileged: None,
            admin_scopes: None,
        }),
        artifacts: Some(token),
    })
}

/// Basic-auth admin token guard.
///
/// Resolves the presented credentials against the [`AdminTokenRegistry`]
/// built from `PDS_ADMIN_TOKEN_<NAME>_*` env vars (and the legacy
/// `PDS_ADMIN_PASSWORD` fallback via [`admin_password_from_env`]).
/// The granted [`AdminScopeSet`] is propagated on the returned
/// `Credentials.admin_scopes` so the route extractors can check the
/// appropriate scope (Invite / Account / Takedown).
pub async fn verify_admin_token(auth_header: Option<&str>) -> Result<AccessOutput, AuthError> {
    let parsed = match parse_basic_auth(auth_header.unwrap_or_default()) {
        None => return Err(AuthError::AuthRequired("AuthMissing".to_string())),
        Some(parsed) => parsed,
    };
    let registry = crate::account::helpers::admin_tokens::AdminTokenRegistry::from_env();
    let Some(scopes) = registry.lookup(&parsed.username, &parsed.password).cloned() else {
        tracing::error!("admin token lookup failed (no matching entry)");
        return Err(AuthError::AuthRequired("BadAuth".to_string()));
    };
    Ok(AccessOutput {
        credentials: Some(Credentials {
            r#type: "admin_token".to_string(),
            did: None,
            scope: None,
            audience: None,
            token_id: None,
            aud: None,
            iss: None,
            is_privileged: None,
            admin_scopes: Some(scopes),
        }),
        artifacts: None,
    })
}

/// User-did (service) JWT guard.
pub async fn verify_user_did_token(auth_header: Option<&str>) -> Result<AccessOutput, AuthError> {
    let Some(token) = bearer_token_from_req(auth_header).ok().flatten() else {
        return Err(AuthError::AuthRequired("AuthMissing".to_string()));
    };
    let Some(resolver) = SIGNING_KEY_RESOLVER.get() else {
        return Err(AuthError::InternalServerError(
            "signing key resolver is not registered".to_string(),
        ));
    };
    let get_signing_key = |iss: String, force_refresh: bool| resolver(iss, force_refresh);
    let payload = verify_service_jwt(
        &token,
        ServiceJwtOpts {
            aud: Some(service_did()),
            iss: None,
        },
        get_signing_key,
    )
    .map_err(|error| AuthError::BadJwt(error.to_string()))?;
    Ok(AccessOutput {
        credentials: Some(Credentials {
            r#type: "user_did".to_string(),
            did: None,
            scope: None,
            audience: None,
            token_id: None,
            aud: Some(payload.aud),
            iss: Some(payload.iss),
            is_privileged: None,
            admin_scopes: None,
        }),
        artifacts: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::auth::CustomClaimObj;
    use std::time::Duration as StdDuration;

    fn setup_env() {
        let defaults = [
            ("PDS_SERVICE_DID", "did:web:localho.st"),
            (
                "PDS_JWT_KEY_K256_PRIVATE_KEY_HEX",
                "9d5907143471e8f0e8df0f8b9512a8c5377878ee767f18fcf961055ecfc071cd",
            ),
            (
                "PDS_REPO_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
                "71cfcf4882a6cff494c3d0affadd3858eb3a5838e7b5e15170e696a590a4fa01",
            ),
        ];
        for (key, value) in defaults {
            if std::env::var(key).is_err() {
                unsafe { std::env::set_var(key, value) };
            }
        }
    }

    fn sign_token(scope: AuthScope, expires_in: StdDuration, subject: &str) -> String {
        let claims = Claims::with_custom_claims(
            CustomClaimObj {
                scope: scope.as_str().to_owned(),
            },
            expires_in.into(),
        )
        .with_audience("did:web:localho.st")
        .with_subject(subject);
        PDS_JWT_KEYPAIR.sign(claims).unwrap()
    }

    fn verify_options() -> VerificationOptions {
        VerificationOptions {
            allowed_audiences: Some(HashSet::from_strings(&["did:web:localho.st".to_string()])),
            time_tolerance: Some(jwt_simple::prelude::Duration::from_secs(0)),
            ..Default::default()
        }
    }

    #[test]
    fn verify_access_token_roundtrip() {
        setup_env();
        let token = sign_token(
            AuthScope::Access,
            StdDuration::from_secs(7200),
            "did:plc:alice",
        );
        let payload = verify_jwt(&token, Some(verify_options())).unwrap();
        assert_eq!(payload.scope, AuthScope::Access);
        assert_eq!(payload.sub.as_deref(), Some("did:plc:alice"));
        assert!(payload.jti.is_none());
        let bearer =
            validate_bearer_token(&token, vec![AuthScope::Access], Some(verify_options())).unwrap();
        assert_eq!(bearer.did, "did:plc:alice");
        assert_eq!(bearer.scope, AuthScope::Access);
        assert_eq!(bearer.audience.as_deref(), Some("did:web:localho.st"));
    }

    #[tokio::test]
    async fn expired_token_surfaces_as_expired() {
        setup_env();
        // Issue a token with a 1-second expiry and wait for it to pass.
        let token = sign_token(
            AuthScope::Access,
            StdDuration::from_secs(1),
            "did:plc:alice",
        );
        tokio::time::sleep(StdDuration::from_secs(2)).await;
        let err = verify_jwt(&token, Some(verify_options())).unwrap_err();
        assert!(is_expired_jwt(&err));
        let err = validate_bearer_token(&token, vec![AuthScope::Access], Some(verify_options()))
            .unwrap_err();
        assert!(is_expired_jwt(&err));
    }

    #[test]
    fn wrong_scope_rejected() {
        setup_env();
        let token = sign_token(
            AuthScope::Refresh,
            StdDuration::from_secs(7200),
            "did:plc:alice",
        );
        let err = validate_bearer_token(&token, vec![AuthScope::Access], Some(verify_options()))
            .unwrap_err();
        assert_eq!(err.to_string(), "Bad token scope");
    }

    #[test]
    fn oauth_scope_mapping_follows_transition_semantics() {
        let scopes = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        assert_eq!(
            oauth_scopes_to_auth_scope(&scopes(&["atproto", "transition:generic"])).unwrap(),
            AuthScope::AppPass
        );
        assert_eq!(
            oauth_scopes_to_auth_scope(&scopes(&[
                "atproto",
                "transition:generic",
                "transition:chat.bsky"
            ]))
            .unwrap(),
            AuthScope::AppPassPrivileged
        );
        assert_eq!(
            oauth_scopes_to_auth_scope(&scopes(&["atproto", "transition:chat.bsky"])).unwrap(),
            AuthScope::AppPassPrivileged
        );
        assert!(oauth_scopes_to_auth_scope(&scopes(&["atproto"])).is_err());
        assert!(oauth_scopes_to_auth_scope(&scopes(&["transition:generic"])).is_err());
        assert!(oauth_scopes_to_auth_scope(&scopes(&[])).is_err());
    }

    fn assert_admin(parsed: Option<BasicAuth>) {
        let parsed = parsed.expect("expected successful parse");
        assert_eq!(parsed.username, "admin");
        assert_eq!(parsed.password, "password");
    }

    #[test]
    fn parses_normal_basic_auth() {
        assert_admin(parse_basic_auth("Basic YWRtaW46cGFzc3dvcmQ="));
    }

    #[test]
    fn tolerates_extra_whitespace() {
        assert_admin(parse_basic_auth("Basic  YWRtaW46cGFzc3dvcmQ="));
        assert_admin(parse_basic_auth("Basic \t YWRtaW46cGFzc3dvcmQ="));
    }

    #[test]
    fn preserves_colons_in_password() {
        let parsed = parse_basic_auth("Basic YWRtaW46cGFzczp3b3Jk").expect("expected parse");
        assert_eq!(parsed.username, "admin");
        assert_eq!(parsed.password, "pass:word");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_basic_auth("").is_none());
        assert!(parse_basic_auth("Basic").is_none());
        assert!(parse_basic_auth("Basic not-base64!").is_none());
        assert!(parse_basic_auth("Bearer YWRtaW46cGFzc3dvcmQ=").is_none());
        assert!(parse_basic_auth("Basic YWRtaW46cGFzc3dvcmQ= extra").is_none());
        assert!(parse_basic_auth("Basic bm8tY29sb24=").is_none());
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn validate_access_token_bearer_roundtrip() {
        setup_env();
        let token = sign_token(
            AuthScope::Access,
            StdDuration::from_secs(7200),
            "did:plc:alice",
        );
        let access = validate_access_token(
            Some(&format!("Bearer {token}")),
            vec![AuthScope::Access],
            None,
        )
        .await
        .unwrap();
        let credentials = access.credentials.unwrap();
        assert_eq!(credentials.r#type, "access");
        assert_eq!(credentials.did.as_deref(), Some("did:plc:alice"));
        assert_eq!(credentials.scope, Some(AuthScope::Access));
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn validate_access_token_expired_maps_to_expired_token() {
        setup_env();
        // Exercise the ExpiredToken branch via the underlying bearer
        // validator with a tight time tolerance; `validate_access_token`
        // uses the production 15-minute tolerance so a 2-second sleep is
        // insufficient there.
        let token = sign_token(
            AuthScope::Access,
            StdDuration::from_secs(1),
            "did:plc:alice",
        );
        tokio::time::sleep(StdDuration::from_secs(2)).await;
        let err = validate_bearer_token(&token, vec![AuthScope::Access], Some(verify_options()))
            .unwrap_err();
        assert!(is_expired_jwt(&err));
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn validate_access_token_records_auth_stage_timing() {
        cacos_pds_core::observability::metrics::init_metrics();
        setup_env();
        let token = sign_token(
            AuthScope::Access,
            StdDuration::from_secs(60),
            "did:plc:alice",
        );
        let header = format!("Bearer {token}");
        let _ = validate_access_token(Some(&header), vec![AuthScope::Access], None).await;
        let snapshot = cacos_pds_core::observability::metrics::render();
        assert!(
            snapshot.contains("cacos_timing_seconds_count{stage=\"auth\"}"),
            "expected auth stage sample: {snapshot}"
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn validate_access_token_missing_bearer_is_auth_required() {
        setup_env();
        let err = validate_access_token(None, vec![AuthScope::Access], None)
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::AuthRequired(_)));
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn validate_refresh_token_accepts_refresh_jwt() {
        setup_env();
        let claims = Claims::with_custom_claims(
            CustomClaimObj {
                scope: AuthScope::Refresh.as_str().to_owned(),
            },
            StdDuration::from_secs(90 * 24 * 60 * 60).into(),
        )
        .with_audience("did:web:localho.st")
        .with_subject("did:plc:alice")
        .with_jwt_id("refresh-tok-1");
        let token = PDS_JWT_KEYPAIR.sign(claims).unwrap();
        let validated = validate_refresh_token(Some(&format!("Bearer {token}")))
            .await
            .unwrap();
        assert_eq!(validated.did, "did:plc:alice");
        assert_eq!(validated.scope, AuthScope::Refresh);
        assert_eq!(validated.payload.jti.as_deref(), Some("refresh-tok-1"));
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn validate_refresh_token_rejects_access_token() {
        setup_env();
        let token = sign_token(
            AuthScope::Access,
            StdDuration::from_secs(7200),
            "did:plc:alice",
        );
        let err = validate_refresh_token(Some(&format!("Bearer {token}")))
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::BadJwt(_)));
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn verify_admin_token_accepts_configured_password() {
        setup_env();
        // SAFETY: tests run sequentially within a single process.
        unsafe {
            std::env::set_var("PDS_ADMIN_PASSWORD", "password");
        }
        let access = verify_admin_token(Some("Basic YWRtaW46cGFzc3dvcmQ="))
            .await
            .unwrap();
        assert_eq!(access.credentials.unwrap().r#type, "admin_token");
        assert!(
            verify_admin_token(Some("Basic YWRtaW46d3Jvbmc="))
                .await
                .is_err()
        );
        assert!(verify_admin_token(None).await.is_err());
        // SAFETY: see above.
        unsafe {
            std::env::remove_var("PDS_ADMIN_PASSWORD");
        }
    }

    #[allow(dead_code)]
    fn _base64_helper_for_quick_check() {
        // Used during development to double-check the parsed basic-auth
        // encoding. Not exercised at runtime.
        let _: String = base64_url::encode("admin:password");
    }
}
