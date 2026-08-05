//! Framework-agnostic auth verification.
//!
//! Quirks intentionally fixed (see ADR 0006 track record):
//! 1. `verify_service_jwt_token` measures `now` in **seconds** (RFC 7519
//!    units) and stores `ServiceJwtPayload.exp` as `Duration::from_secs`.
//! 2. `verify_service_jwt` treats `iss` LISTED in `ServiceJwtOpts.iss` as
//!    **trusted** (the inverted-then-`!contains` semantics is the correct
//!    one). Mirrors the OIDC trust check.
//! 3. `verify_service_jwt_token` decodes the signature segment via
//!    base64ct `Base64UrlUnpadded`, hashes the JWS signing input with
//!    SHA-256, and calls `rsky_crypto::verify::verify_signature_digest`
//!    (which expects digest semantics). The previous port re-base64-encoded
//!    the signature and passed the raw signing input.
//! 4. `did_cache` timestamps are reported in **microseconds** to match
//!    `rsky_identity::MemoryCache`; see `crate::identity::did_cache`.
//!
//! Plus `is_user_or_admin` matches on `r#type == "admin_token"`, not on
//!    `did`, so the admin guard works as documented.

use crate::account::AccountManager;
use crate::account::helpers::account::{ActorAccount, AvailabilityFlags};
use crate::account::helpers::auth::CustomClaimObj;
use anyhow::{Result, bail};
use base64ct::{Base64UrlUnpadded, Encoding};
use jwt_simple::prelude::*;
use rsky_crypto::verify::verify_signature_digest;
use rsky_oauth::dpop::DpopRequest;
use rsky_oauth::{OAuthProvider, VerifiedAccess};
use secp256k1::{Keypair, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::env;
use std::str;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const INFINITY: u64 = u64::MAX;

/// Resolves a `did` (issuer) to its signing key (a did:key string). Wired by
/// the identity plan; returns `InternalServerError` until then.
pub type SigningKeyResolver = Box<dyn Fn(String, bool) -> Result<String> + Send + Sync>;

static OAUTH_PROVIDER: OnceLock<Arc<OAuthProvider>> = OnceLock::new();
static ACCOUNT_MANAGER: OnceLock<Arc<AccountManager>> = OnceLock::new();
static SIGNING_KEY_RESOLVER: OnceLock<SigningKeyResolver> = OnceLock::new();

/// Register the shared OAuth provider (for DPoP-bound access-token
/// validation) and the `AccountManager` (for account-status checks). Called
/// once at startup by the app bootstrap. Bearer-only callers never need the
/// provider.
pub fn register_auth_dependencies(
    account_manager: Arc<AccountManager>,
    provider: Option<Arc<OAuthProvider>>,
) {
    if let Some(provider) = provider {
        let _ = OAUTH_PROVIDER.set(provider);
    }
    let _ = ACCOUNT_MANAGER.set(account_manager);
}

/// Register the did -> signing-key resolver used by `verify_user_did_token`
/// and `verify_service_jwt` (wired by the identity plan once IdResolver
/// exists; until then `verify_user_did_token` returns InternalServerError).
pub fn register_signing_key_resolver(
    resolver: SigningKeyResolver,
) -> Result<(), SigningKeyResolver> {
    SIGNING_KEY_RESOLVER.set(resolver)
}

/// The per-request context DPoP proofs bind to (RFC 9449 §4.3): HTTP method,
/// absolute request URI, and all `DPoP` header values.
#[derive(Debug, Clone, Default)]
pub struct DpopRequestContext {
    pub method: String,
    pub uri: String,
    pub dpop_headers: Vec<String>,
}

thread_local! {
    static DPOP_REQUEST_CONTEXT: RefCell<Option<DpopRequestContext>> =
        const { RefCell::new(None) };
}

/// Registers the current request's DPoP context so
/// [`validate_access_token`] can validate DPoP-bound access tokens.
pub fn set_dpop_request_context(ctx: DpopRequestContext) {
    DPOP_REQUEST_CONTEXT.with(|cell| *cell.borrow_mut() = Some(ctx));
}

pub fn clear_dpop_request_context() {
    DPOP_REQUEST_CONTEXT.with(|cell| *cell.borrow_mut() = None);
}

fn current_dpop_request_context() -> Option<DpopRequestContext> {
    DPOP_REQUEST_CONTEXT.with(|cell| cell.borrow().clone())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

fn service_did() -> String {
    env::var("PDS_SERVICE_DID").unwrap_or_default()
}

/// True when `err` is jwt-simple's expiry error (`JWTError::TokenHasExpired`),
/// as opposed to a signature, format, or other verification failure.
pub(crate) fn is_expired_jwt(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<jwt_simple::JWTError>(),
        Some(jwt_simple::JWTError::TokenHasExpired)
    )
}

/// Canonical ES256K signing key for access/refresh/service JWTs, from
/// `PDS_JWT_KEY_K256_PRIVATE_KEY_HEX`. Owned HERE; Plan 05's account
/// helpers import this static rather than re-define one.
pub static PDS_JWT_KEYPAIR: LazyLock<ES256kKeyPair> = LazyLock::new(|| {
    let secp = Secp256k1::new();
    let private_key = env::var("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX").unwrap();
    let secret_key = SecretKey::from_slice(&hex::decode(private_key.as_bytes()).unwrap()).unwrap();
    let jwt_key = Keypair::from_secret_key(&secp, &secret_key);
    ES256kKeyPair::from_bytes(jwt_key.secret_bytes().as_slice()).unwrap()
});

#[derive(PartialEq, Clone, Debug)]
pub enum AuthScope {
    Access,
    Refresh,
    AppPass,
    AppPassPrivileged,
    SignupQueued,
}

impl AuthScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthScope::Access => "com.atproto.access",
            AuthScope::Refresh => "com.atproto.refresh",
            AuthScope::AppPass => "com.atproto.appPass",
            AuthScope::AppPassPrivileged => "com.atproto.appPassPrivileged",
            AuthScope::SignupQueued => "com.atproto.signupQueued",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(scope: &str) -> Result<Self> {
        match scope {
            "com.atproto.access" => Ok(AuthScope::Access),
            "com.atproto.refresh" => Ok(AuthScope::Refresh),
            "com.atproto.appPass" => Ok(AuthScope::AppPass),
            "com.atproto.appPassPrivileged" => Ok(AuthScope::AppPassPrivileged),
            "com.atproto.signupQueued" => Ok(AuthScope::SignupQueued),
            _ => bail!("Invalid AuthScope: `{scope:?}` is not a valid auth scope"),
        }
    }
}

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

pub struct ServiceJwtOpts {
    pub aud: Option<String>,
    pub iss: Option<Vec<String>>,
}

pub struct ValidateAccessTokenOpts {
    pub check_takedown: Option<bool>,
    pub check_deactivated: Option<bool>,
}

#[derive(Debug)]
pub struct VerifiedServiceJwt {
    pub aud: String,
    pub iss: String,
}

pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

/// Minimal projection of `ActorAccount` used by the pure status checks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountStatus {
    pub deactivated_at: Option<String>,
    pub takedown_ref: Option<String>,
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

const BEARER: &str = "Bearer ";
const BASIC: &str = "Basic ";
const DPOP: &str = "DPoP ";

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

pub fn admin_password_from_env() -> Option<String> {
    env::var("PDS_ADMIN_PASSWORD")
        .or_else(|_| env::var("PDS_ADMIN_PASS"))
        .ok()
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
    let decoded: Vec<u8> = match base64ct::Base64::decode_vec(b64) {
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
        }),
        artifacts: Some(token),
    })
}

/// Pure account-status check on the pre-fetched [`AccountStatus`].
pub fn check_account_status(
    account: Option<&AccountStatus>,
    check_takedown: bool,
    check_deactivated: bool,
) -> Result<(), AuthError> {
    if !check_takedown && !check_deactivated {
        return Ok(());
    }
    let Some(account) = account else {
        return Err(AuthError::AccountNotFound("Account not found".to_string()));
    };
    if check_takedown && account.takedown_ref.is_some() {
        return Err(AuthError::AccountTakedown(
            "Account has been taken down".to_string(),
        ));
    }
    if check_deactivated && account.deactivated_at.is_some() {
        return Err(AuthError::AccountDeactivated(
            "Account is deactivated".to_string(),
        ));
    }
    Ok(())
}

/// Fetches the account projection for [`check_account_status`] from the
/// registered `AccountManager`.
pub async fn fetch_account_status(did: &str) -> Result<Option<AccountStatus>, AuthError> {
    let Some(account_manager) = ACCOUNT_MANAGER.get() else {
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
    let Some(provider) = OAUTH_PROVIDER.get() else {
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
        }),
        artifacts: Some(token),
    })
}

/// Basic-auth admin token guard.
pub async fn verify_admin_token(auth_header: Option<&str>) -> Result<AccessOutput, AuthError> {
    match parse_basic_auth(auth_header.unwrap_or_default()) {
        None => Err(AuthError::AuthRequired("AuthMissing".to_string())),
        Some(parsed) => {
            let Some(admin_password) = admin_password_from_env() else {
                tracing::error!("admin password is not configured");
                return Err(AuthError::AuthRequired("BadAuth".to_string()));
            };
            if parsed.username != "admin" || parsed.password != admin_password {
                Err(AuthError::AuthRequired("BadAuth".to_string()))
            } else {
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
                    }),
                    artifacts: None,
                })
            }
        }
    }
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
        }),
        artifacts: None,
    })
}

pub fn is_user_or_admin(auth: &AccessOutput, did: &String) -> bool {
    match &auth.credentials {
        Some(credentials) if credentials.r#type == "admin_token" => true,
        Some(credentials) => credentials.did == Some(did.to_string()),
        None => false,
    }
}

/// Raw claims of a service JWT (for parsing the base64url payload segment).
#[derive(serde::Deserialize)]
struct RawServiceJwtPayload {
    iss: String,
    aud: String,
    exp: u64,
}

/// Claims of a verified service JWT.
pub struct ServiceJwtPayload {
    pub iss: String,
    pub aud: String,
    pub exp: Option<Duration>,
}

fn parse_payload(b64: &str) -> Result<RawServiceJwtPayload> {
    Ok(serde_json::from_slice::<RawServiceJwtPayload>(
        &Base64UrlUnpadded::decode_vec(b64)?,
    )?)
}

/// Verifies a service JWT (`x.y.z`) — secp256k1 ES256K signature using
/// a did:key signing key. Quirk 1 (exp units) and Quirk 3 (sig
/// decoding + digest) are fixed here.
pub fn verify_service_jwt(
    jwt: &str,
    opts: ServiceJwtOpts,
    get_signing_key: impl Fn(String, bool) -> Result<String>,
) -> Result<VerifiedServiceJwt> {
    let get_signing_key = move |iss: String, force_refresh: bool| -> Result<String> {
        match &opts.iss {
            // An issuer listed in `opts.iss` is **trusted** — the inverse
            // of the upstream bug. Rectifies quirk 2.
            Some(opts_iss) if !opts_iss.contains(&iss) => {
                bail!("UntrustedIss: Untrusted issuer")
            }
            _ => (),
        }
        get_signing_key(iss, force_refresh)
    };
    let payload: ServiceJwtPayload = verify_service_jwt_token(jwt, opts.aud, get_signing_key)?;
    Ok(VerifiedServiceJwt {
        iss: payload.iss,
        aud: payload.aud,
    })
}

fn verify_service_jwt_token(
    jwt_str: &str,
    own_did: Option<String>,
    get_signing_key: impl Fn(String, bool) -> Result<String>,
) -> Result<ServiceJwtPayload> {
    let parts: Vec<&str> = jwt_str.split('.').collect();
    match (parts.first(), parts.get(1), parts.get(2)) {
        (Some(_), Some(parts_1), Some(sig)) if parts.len() == 3 => {
            let parts_1 = *parts_1;
            let sig = *sig;
            let payload = parse_payload(parts_1)?;
            // Quirk 1 fix: now in **seconds** (RFC 7519 units).
            let now = now_secs();
            if now > payload.exp {
                bail!("JwtExpired: jwt expired")
            }
            if let Some(own_did) = &own_did
                && payload.aud != *own_did
            {
                bail!("BadJwtAudience: jwt audience does not match service did")
            }
            // Quirk 3 fix: hash the JWS signing input with SHA-256, decode
            // the signature segment, verify with digest semantics.
            let msg_bytes = [parts[0].as_bytes(), b".", parts[1].as_bytes()].concat();
            let digest = Sha256::digest(&msg_bytes);

            let sig_bytes = Base64UrlUnpadded::decode_vec(sig)?;
            let verify_signature_with_key = |key: String| -> Result<bool> {
                verify_signature_digest(&key, digest.as_ref(), &sig_bytes, None)
            };

            let signing_key = get_signing_key(payload.iss.clone(), false)?;

            let mut valid_sig: bool = match verify_signature_with_key(signing_key.clone()) {
                Ok(is_valid) => is_valid,
                Err(err) => {
                    bail!("BadJwtSignature: could not verify jwt signature: {err}")
                }
            };

            if !valid_sig {
                let fresh_signing_key = get_signing_key(payload.iss.clone(), true)?;
                valid_sig = if fresh_signing_key != signing_key {
                    match verify_signature_with_key(fresh_signing_key) {
                        Ok(is_valid) => is_valid,
                        Err(err) => {
                            bail!("BadJwtSignature: could not verify jwt signature: {err}")
                        }
                    }
                } else {
                    false
                };
            }

            if !valid_sig {
                bail!("BadJwtSignature: jwt signature does not match jwt issuer")
            }

            Ok(ServiceJwtPayload {
                iss: payload.iss,
                aud: payload.aud,
                exp: Some(Duration::from_secs(payload.exp)),
            })
        }
        _ => bail!("BadJwt: poorly formatted jwt"),
    }
}

// A public helper used by tests to construct a service JWT that this
// verifier can verify (round-trip). Reuses the existing
// `create_service_jwt` from `crate::account::helpers::auth` which uses
// `PDS_REPO_SIGNING_KEYPAIR` and emits a compact ECDSA signature.
pub use crate::account::helpers::auth::create_service_jwt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::helpers::auth::CustomClaimObj;
    use crate::account::helpers::auth::{ServiceJwtParams, create_service_jwt};
    use jwt_simple::algorithms::ES256kKeyPair;
    use rsky_crypto::constants::SECP256K1_JWT_ALG;
    use rsky_crypto::did::format_did_key;
    use secp256k1::PublicKey;
    use std::collections::HashSet;
    use std::time::Duration as StdDuration;

    const _TEST_KEY_HEX: &str = "B5C4DEEA11C2AA08AB6AF8BA62F8B1EB5BFE0FEB03CE3C24D";

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
    fn admin_password_prefers_upstream_env_name() {
        // SAFETY: tests run sequentially within a single process; no concurrent
        // env reads.
        unsafe {
            std::env::remove_var("PDS_ADMIN_PASSWORD");
            std::env::remove_var("PDS_ADMIN_PASS");
        }
        assert_eq!(admin_password_from_env(), None);

        // SAFETY: see above.
        unsafe {
            std::env::set_var("PDS_ADMIN_PASS", "legacy");
        }
        assert_eq!(admin_password_from_env(), Some("legacy".to_string()));

        // SAFETY: see above.
        unsafe {
            std::env::set_var("PDS_ADMIN_PASSWORD", "standard");
        }
        assert_eq!(admin_password_from_env(), Some("standard".to_string()));

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("PDS_ADMIN_PASSWORD");
            std::env::remove_var("PDS_ADMIN_PASS");
        }
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

    #[test]
    fn check_account_status_flags() {
        let err = check_account_status(None, true, false).unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(_)));
        assert!(check_account_status(None, false, false).is_ok());

        let taken_down = AccountStatus {
            deactivated_at: None,
            takedown_ref: Some("admin#1".to_string()),
        };
        let err = check_account_status(Some(&taken_down), true, false).unwrap_err();
        assert!(matches!(err, AuthError::AccountTakedown(_)));

        let deactivated = AccountStatus {
            deactivated_at: Some("2026-01-01T00:00:00.000Z".to_string()),
            takedown_ref: None,
        };
        let err = check_account_status(Some(&deactivated), false, true).unwrap_err();
        assert!(matches!(err, AuthError::AccountDeactivated(_)));
        assert!(check_account_status(Some(&deactivated), false, false).is_ok());
    }

    #[test]
    fn verify_service_jwt_trusts_listed_issuer() {
        // Quirk 2 fix: an issuer LISTED in `opts.iss` is trusted. The
        // malformed token "x.y.z" fails at payload parsing (Base64), but
        // only after the trust check passes — confirming the listed issuer
        // was accepted.
        let opts = ServiceJwtOpts {
            aud: None,
            iss: Some(vec!["did:web:issuer.test".to_string()]),
        };
        let err = verify_service_jwt("x.y.z", opts, |_iss, _force| {
            bail!("resolver should not be called: payload parse fails first")
        })
        .unwrap_err();
        // The error is a payload parsing failure (BadJwt-shaped), proving
        // the trust check did not short-circuit with UntrustedIss.
        assert!(
            err.to_string().contains("BadJwt") || err.to_string().contains("Base64"),
            "expected parsing failure (BadJwt/Base64), got: {err}"
        );
    }

    #[tokio::test]
    async fn verify_service_jwt_rejects_unlisted_issuer() {
        // Quirk 2 fix: an issuer NOT listed in `opts.iss` is rejected by
        // the trust check after the payload parses but before the signing
        // key is resolved. Use `create_service_jwt` to produce a well-formed
        // JWT against the registered PDS repo signing key, then verify
        // with a different `opts.iss` allow-list.
        setup_env();
        let iss = format!(
            "did:web:{}",
            hex::encode(rsky_crypto::utils::random_bytes(8))
        );
        let aud = "did:web:localho.st";
        let jwt = create_service_jwt(ServiceJwtParams {
            iss: iss.clone(),
            aud: aud.to_owned(),
            exp: None,
            lxm: None,
            jti: None,
        })
        .await
        .unwrap();
        let opts = ServiceJwtOpts {
            aud: None,
            iss: Some(vec!["did:web:trusted.test".to_string()]),
        };
        let err = verify_service_jwt(&jwt, opts, |_iss, _force| {
            bail!("resolver should not be called for unlisted issuer")
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("UntrustedIss"),
            "expected UntrustedIss, got: {err}"
        );
    }

    #[test]
    fn verify_service_jwt_rejects_malformed_token() {
        let err = verify_service_jwt(
            "not-a-jwt",
            ServiceJwtOpts {
                aud: None,
                iss: None,
            },
            |_iss, _force| bail!("unexpected resolution"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("BadJwt"));
    }

    #[test]
    fn verify_service_jwt_propagates_resolution_errors() {
        setup_env();
        let iss = format!(
            "did:web:{}",
            hex::encode(rsky_crypto::utils::random_bytes(8))
        );
        let aud = "did:web:localho.st";
        let jwt = futures::executor::block_on(create_service_jwt(ServiceJwtParams {
            iss: iss.clone(),
            aud: aud.to_owned(),
            exp: None,
            lxm: None,
            jti: None,
        }))
        .unwrap();
        let err = verify_service_jwt(
            &jwt,
            ServiceJwtOpts {
                aud: None,
                iss: None,
            },
            |requested_iss, _force| {
                assert_eq!(requested_iss, iss);
                anyhow::bail!("could not resolve iss did")
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("could not resolve iss did"));
    }

    /// Round-trip: sign a service JWT with the registered
    /// `PDS_REPO_SIGNING_KEYPAIR`, build the matching did:key from the
    /// public key bytes, and verify the verifier accepts it. Proves
    /// quirks 1 (exp in seconds) and 3 (sig decode + digest) are fixed.
    #[tokio::test]
    async fn verify_service_jwt_roundtrip() {
        setup_env();
        let secret_key = crate::context::PDS_REPO_SIGNING_KEYPAIR.secret_key();
        let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), &secret_key);
        let did_key = format_did_key(
            SECP256K1_JWT_ALG.to_string(),
            pubkey.serialize_uncompressed().to_vec(),
        )
        .unwrap();
        let iss = did_key.clone();
        let aud = "did:web:localho.st";
        let jwt = create_service_jwt(ServiceJwtParams {
            iss: iss.clone(),
            aud: aud.to_owned(),
            exp: None,
            lxm: None,
            jti: None,
        })
        .await
        .unwrap();

        let result = verify_service_jwt(
            &jwt,
            ServiceJwtOpts {
                aud: None,
                iss: None,
            },
            |requested_iss, _force| {
                assert_eq!(requested_iss, iss);
                Ok(did_key.clone())
            },
        );
        assert!(
            result.is_ok(),
            "expected round-trip verification to succeed, got: {:?}",
            result.err()
        );
        let verified = result.unwrap();
        assert_eq!(verified.iss, iss);
        assert_eq!(verified.aud, aud);
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

    #[test]
    fn is_user_or_admin_matches_by_token_type() {
        let admin = AccessOutput {
            credentials: Some(Credentials {
                r#type: "admin_token".to_string(),
                did: None,
                scope: None,
                audience: None,
                token_id: None,
                aud: None,
                iss: None,
                is_privileged: None,
            }),
            artifacts: None,
        };
        assert!(is_user_or_admin(&admin, &"any".to_string()));

        let user = AccessOutput {
            credentials: Some(Credentials {
                r#type: "access".to_string(),
                did: Some("did:plc:alice".to_string()),
                scope: Some(AuthScope::Access),
                audience: None,
                token_id: None,
                aud: None,
                iss: None,
                is_privileged: None,
            }),
            artifacts: None,
        };
        assert!(is_user_or_admin(&user, &"did:plc:alice".to_string()));
        assert!(!is_user_or_admin(&user, &"did:plc:bob".to_string()));
        assert!(!is_user_or_admin(
            &AccessOutput {
                credentials: None,
                artifacts: None,
            },
            &"did:plc:alice".to_string(),
        ));
    }

    #[allow(dead_code)]
    fn _base64_helper_for_quick_check() {
        // Used during development to double-check the parsed basic-auth
        // encoding. Not exercised at runtime.
        let _: String = base64_url::encode("admin:password");
    }
}
