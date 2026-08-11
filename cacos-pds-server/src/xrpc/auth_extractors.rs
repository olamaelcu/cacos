//! Poem auth extractors.
//!
//! Each extractor reads the `Authorization` header from the incoming
//! poem [`Request`], calls the pure verifier in
//! [`crate::auth::auth_verifier`], and converts [`AuthError`] into
//! [`ApiError`] (then into a [`poem::Error`] via the `From<ApiError> for
//! poem::Error` impl defined in `crate::xrpc::error`). The scope matrix
//! mirrors the olamaelcu/rsky `src/auth_verifier.rs` referenced by the
//! plan: each extractor pins the [`AuthScope`] subset it accepts.
//!
//! Plan 06 contract: `validate_access_token` reads the DPoP request
//! context from a thread-local, so every access-token validation is
//! wrapped in `set_dpop_request_context` / `clear_dpop_request_context`.

use cacos_pds_account::account::helpers::admin_tokens::AdminScope;
use cacos_pds_account::account::helpers::auth::AuthScope;
use cacos_pds_account::auth::verifier::{
    AccessOutput, AuthError, Credentials, DpopRequestContext, ValidateAccessTokenOpts,
};
use crate::xrpc::ApiError;
use poem::IntoResponse;
use poem::Request;
use poem::RequestBody;
use poem::Result;
use poem::error::Error as PoemError;
use poem::web::FromRequest;

/// Maps an auth failure to the wire-facing [`ApiError`].
///
/// [`AuthError::ExpiredToken`] and [`AuthError::AuthRequired`] map to
/// their dedicated variants so the status code and `error` field match
/// the XRPC contract (ExpiredToken stays as the well-known `ExpiredToken`,
/// AuthRequired produces the 401 `AuthRequiredError` shape). Everything
/// else falls through to `ApiError::InvalidRequest` with the
/// `thiserror` `Display` text.
fn auth_error_to_api_error(error: &AuthError) -> ApiError {
    match error {
        AuthError::ExpiredToken => ApiError::ExpiredToken,
        AuthError::AuthRequired(message) => ApiError::AuthRequiredError(message.clone()),
        other => ApiError::InvalidRequest(other.to_string()),
    }
}

fn poem_error(api: ApiError) -> PoemError {
    PoemError::from_response(api.into_response())
}

fn auth_header(req: &Request) -> Option<&str> {
    req.headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
}

/// Plan 06 contract: the pure `validate_access_token` reads the DPoP
/// request context from a thread-local set by the poem layer. Wrap every
/// access-token validation with set/clear scoped to this request. (The
/// absolute-URI form required by RFC 9449 §4.3 can be completed in the
/// OAuth sweep by prefixing `req.uri()` with `config.service.public_url`
/// via `req.data::<SharedState>()`.)
async fn validate_access(
    req: &Request,
    scopes: Vec<AuthScope>,
    opts: Option<ValidateAccessTokenOpts>,
) -> std::result::Result<AccessOutput, AuthError> {
    cacos_pds_account::auth::verifier::set_dpop_request_context(DpopRequestContext {
        method: req.method().to_string(),
        uri: req.uri().to_string(),
        dpop_headers: req
            .headers()
            .get_all("dpop")
            .iter()
            .filter_map(|v| v.to_str().ok().map(String::from))
            .collect(),
    });
    let result = cacos_pds_account::auth::verifier::validate_access_token(auth_header(req), scopes, opts).await;
    cacos_pds_account::auth::verifier::clear_dpop_request_context();
    result
}

pub struct AccessStandard {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AccessStandard {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let scopes = vec![
            AuthScope::Access,
            AuthScope::AppPass,
            AuthScope::AppPassPrivileged,
        ];
        match validate_access(req, scopes, None).await {
            Ok(access) => Ok(AccessStandard { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

pub struct AccessFull {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AccessFull {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let scopes = vec![AuthScope::Access];
        match validate_access(req, scopes, None).await {
            Ok(access) => Ok(AccessFull { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

pub struct AccessPrivileged {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AccessPrivileged {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let scopes = vec![AuthScope::Access, AuthScope::AppPassPrivileged];
        match validate_access(req, scopes, None).await {
            Ok(access) => Ok(AccessPrivileged { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// AccessStandard + account-status checks (deactivated/takedown).
pub struct AccessStandardIncludeChecks {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AccessStandardIncludeChecks {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let scopes = vec![
            AuthScope::Access,
            AuthScope::AppPass,
            AuthScope::AppPassPrivileged,
        ];
        let opts = Some(ValidateAccessTokenOpts {
            check_deactivated: Some(true),
            check_takedown: Some(true),
        });
        match validate_access(req, scopes, opts).await {
            Ok(access) => Ok(AccessStandardIncludeChecks { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// AccessStandard + takedown check only.
pub struct AccessStandardCheckTakedown {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AccessStandardCheckTakedown {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let scopes = vec![
            AuthScope::Access,
            AuthScope::AppPass,
            AuthScope::AppPassPrivileged,
        ];
        let opts = Some(ValidateAccessTokenOpts {
            check_deactivated: None,
            check_takedown: Some(true),
        });
        match validate_access(req, scopes, opts).await {
            Ok(access) => Ok(AccessStandardCheckTakedown { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// Full access, but deactivated accounts may still import.
pub struct AccessFullImport {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AccessFullImport {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let scopes = vec![AuthScope::Access];
        let opts = Some(ValidateAccessTokenOpts {
            check_deactivated: Some(false),
            check_takedown: Some(true),
        });
        match validate_access(req, scopes, opts).await {
            Ok(access) => Ok(AccessFullImport { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// Access, AppPass, AppPassPrivileged, or SignupQueued — used by temp.checkSignupQueue.
pub struct AccessStandardSignupQueued {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AccessStandardSignupQueued {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let scopes = vec![
            AuthScope::Access,
            AuthScope::AppPass,
            AuthScope::AppPassPrivileged,
            AuthScope::SignupQueued,
        ];
        match validate_access(req, scopes, None).await {
            Ok(access) => Ok(AccessStandardSignupQueued { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

pub struct Refresh {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for Refresh {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        match cacos_pds_account::auth::verifier::validate_refresh_token(auth_header(req)).await {
            Ok(validated) => Ok(Refresh {
                access: AccessOutput {
                    credentials: Some(Credentials {
                        r#type: "refresh".to_string(),
                        did: Some(validated.did),
                        scope: Some(validated.scope),
                        audience: validated.audience,
                        token_id: validated.payload.jti,
                        aud: None,
                        iss: None,
                        is_privileged: None,
                        admin_scopes: None,
                    }),
                    artifacts: Some(validated.token),
                },
            }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// Extracts just the refresh token id — used by server.deleteSession.
pub struct RevokeRefreshToken {
    pub id: String,
}

impl<'a> FromRequest<'a> for RevokeRefreshToken {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        match cacos_pds_account::auth::verifier::validate_refresh_token(auth_header(req)).await {
            Ok(validated) => match validated.payload.jti {
                Some(jti) => Ok(RevokeRefreshToken { id: jti }),
                None => Err(poem_error(ApiError::InvalidRequest(
                    "Unexpected missing refresh token id".to_string(),
                ))),
            },
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// Basic-auth admin token (username `admin` + `PDS_ADMIN_PASSWORD`/`PDS_ADMIN_PASS`).
pub struct AdminToken {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for AdminToken {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        match cacos_pds_account::auth::verifier::verify_admin_token(auth_header(req)).await {
            Ok(access) => Ok(AdminToken { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// Checks the granted admin scopes on `credentials` for `scope`. Returns
/// `AuthRequiredError` when the credentials don't carry the requested
/// scope (either because the credential type isn't admin or because the
/// configured token registry entry doesn't grant it). Wildcard-scope
/// entries grant every checked scope by definition.
fn require_admin_scope(
    credentials: &Option<Credentials>,
    scope: AdminScope,
) -> Result<(), PoemError> {
    let granted = credentials.as_ref().and_then(|c| c.admin_scopes.as_ref());
    match granted {
        Some(scopes) if scopes.contains(scope) => Ok(()),
        _ => Err(poem_error(ApiError::AuthRequiredError(format!(
            "Missing admin scope: {}",
            scope.as_str()
        )))),
    }
}

/// Admin token + `InviteAdmin` scope required. Used by the invite-code
/// issuance endpoints (`com.atproto.server.createInviteCode[s]`).
pub struct RequireInviteAdmin(pub AccessOutput);

impl<'a> FromRequest<'a> for RequireInviteAdmin {
    async fn from_request(req: &'a Request, body: &mut RequestBody) -> Result<Self> {
        let admin = AdminToken::from_request(req, body).await?;
        require_admin_scope(&admin.access.credentials, AdminScope::InviteAdmin)?;
        Ok(Self(admin.access))
    }
}

/// Admin token + `AccountAdmin` scope required. Used by destructive
/// account-management endpoints (`com.atproto.server.deleteAccount`).
pub struct RequireAccountAdmin(pub AccessOutput);

impl<'a> FromRequest<'a> for RequireAccountAdmin {
    async fn from_request(req: &'a Request, body: &mut RequestBody) -> Result<Self> {
        let admin = AdminToken::from_request(req, body).await?;
        require_admin_scope(&admin.access.credentials, AdminScope::AccountAdmin)?;
        Ok(Self(admin.access))
    }
}

/// Admin token + `TakedownAdmin` scope required. Used by takedown / repo
/// moderation endpoints.
pub struct RequireTakedownAdmin(pub AccessOutput);

impl<'a> FromRequest<'a> for RequireTakedownAdmin {
    async fn from_request(req: &'a Request, body: &mut RequestBody) -> Result<Self> {
        let admin = AdminToken::from_request(req, body).await?;
        require_admin_scope(&admin.access.credentials, AdminScope::TakedownAdmin)?;
        Ok(Self(admin.access))
    }
}

/// Either a moderator service token or an admin token.
pub struct Moderator {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for Moderator {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        if auth_header(req)
            .map(|h| h.starts_with("Bearer "))
            .unwrap_or(false)
        {
            // Mod-service JWT verification requires the signing-key resolver
            // (Plan 06 register_signing_key_resolver). Until a mod service is
            // configured (PDS_MOD_SERVICE_DID unset), the bearer branch fails
            // closed; admin basic-auth remains the supported path (Task 26).
            return Err(poem_error(ApiError::InvalidRequest(
                "moderator token not yet wired".to_string(),
            )));
        }
        match cacos_pds_account::auth::verifier::verify_admin_token(auth_header(req)).await {
            Ok(access) => Ok(Moderator { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

/// No auth, access token, or admin token — the optional guard used by sync handlers.
pub struct OptionalAccessOrAdminToken {
    pub access: Option<AccessOutput>,
}

impl<'a> FromRequest<'a> for OptionalAccessOrAdminToken {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        let header = auth_header(req);
        match header {
            None => Ok(OptionalAccessOrAdminToken { access: None }),
            Some(h) if h.starts_with("Bearer ") => {
                match validate_access(req, vec![AuthScope::Access], None).await {
                    Ok(access) => Ok(OptionalAccessOrAdminToken {
                        access: Some(access),
                    }),
                    Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
                }
            }
            Some(_) => match cacos_pds_account::auth::verifier::verify_admin_token(auth_header(req)).await {
                Ok(access) => Ok(OptionalAccessOrAdminToken {
                    access: Some(access),
                }),
                Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
            },
        }
    }
}

/// A user-did (service) JWT — used by createAccount's optional auth.
pub struct UserDidAuthOptional {
    pub access: Option<AccessOutput>,
}

impl<'a> FromRequest<'a> for UserDidAuthOptional {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        if auth_header(req)
            .map(|h| h.starts_with("Bearer "))
            .unwrap_or(false)
        {
            match cacos_pds_account::auth::verifier::verify_user_did_token(auth_header(req)).await {
                Ok(access) => Ok(UserDidAuthOptional {
                    access: Some(access),
                }),
                Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
            }
        } else {
            Ok(UserDidAuthOptional { access: None })
        }
    }
}

/// Required user-did (service) JWT.
pub struct UserDidAuth {
    pub access: AccessOutput,
}

impl<'a> FromRequest<'a> for UserDidAuth {
    async fn from_request(req: &'a Request, _body: &mut RequestBody) -> Result<Self> {
        match cacos_pds_account::auth::verifier::verify_user_did_token(auth_header(req)).await {
            Ok(access) => Ok(UserDidAuth { access }),
            Err(error) => Err(poem_error(auth_error_to_api_error(&error))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xrpc::test_utils::{create_test_account, test_state};
    use poem::test::TestClient;
    use poem::web::Json;
    use poem::{EndpointExt, Route, get, handler};

    #[handler]
    async fn whoami(auth: AccessStandard) -> Json<serde_json::Value> {
        let did = auth.access.credentials.unwrap().did.unwrap();
        Json(serde_json::json!({ "did": did }))
    }

    #[handler]
    async fn full_only(_auth: AccessFull) -> &'static str {
        "ok"
    }

    #[handler]
    async fn needs_refresh(_auth: Refresh) -> &'static str {
        "ok"
    }

    #[tokio::test]
    async fn valid_access_jwt_passes_access_standard() {
        let (state, _dirs) = test_state().await;
        let (access, _refresh) = create_test_account(&state, "did:plc:alice", "alice.test").await;
        let app = Route::new().at("/whoami", get(whoami)).data(state);
        let cli = TestClient::new(app);
        let resp = cli
            .get("/whoami")
            .header("Authorization", format!("Bearer {access}"))
            .send()
            .await;
        assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
        let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
        assert_eq!(body["did"], "did:plc:alice");
    }

    #[tokio::test]
    async fn refresh_jwt_rejected_by_access_standard() {
        let (state, _dirs) = test_state().await;
        let (_access, refresh) = create_test_account(&state, "did:plc:bob", "bob.test").await;
        let app = Route::new().at("/whoami", get(whoami)).data(state);
        let cli = TestClient::new(app);
        let resp = cli
            .get("/whoami")
            .header("Authorization", format!("Bearer {refresh}"))
            .send()
            .await;
        assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
        assert_eq!(body["error"], "InvalidRequest");
    }

    #[tokio::test]
    async fn missing_auth_returns_401_auth_required() {
        let (state, _dirs) = test_state().await;
        let app = Route::new().at("/whoami", get(whoami)).data(state);
        let cli = TestClient::new(app);
        let resp = cli.get("/whoami").send().await;
        assert_eq!(resp.0.status(), poem::http::StatusCode::UNAUTHORIZED);
        let body: serde_json::Value = resp.0.into_body().into_json().await.unwrap();
        assert_eq!(body["error"], "AuthRequiredError");
    }

    #[tokio::test]
    async fn app_password_scope_rejected_by_access_full() {
        let (state, _dirs) = test_state().await;
        let (_access, _refresh) = create_test_account(&state, "did:plc:carol", "carol.test").await;
        // an app password session has AppPass scope; AccessFull requires Access
        let (app_pass_access, _) = state
            .account_manager
            .create_session("did:plc:carol".to_owned(), Some("my app".to_owned()))
            .await
            .unwrap();
        let app = Route::new().at("/full", get(full_only)).data(state);
        let cli = TestClient::new(app);
        let resp = cli
            .get("/full")
            .header("Authorization", format!("Bearer {app_pass_access}"))
            .send()
            .await;
        assert_eq!(resp.0.status(), poem::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn refresh_extractor_accepts_refresh_jwt() {
        let (state, _dirs) = test_state().await;
        let (_access, refresh) = create_test_account(&state, "did:plc:dan", "dan.test").await;
        let app = Route::new().at("/refresh", get(needs_refresh)).data(state);
        let cli = TestClient::new(app);
        let resp = cli
            .get("/refresh")
            .header("Authorization", format!("Bearer {refresh}"))
            .send()
            .await;
        assert_eq!(resp.0.status(), poem::http::StatusCode::OK);
    }
}
