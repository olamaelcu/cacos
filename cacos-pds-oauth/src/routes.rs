//! OAuth provider HTTP routes (poem).
//!
//! Port of `rsky-pds/src/oauth/routes.rs` onto the cacos poem stack,
//! with the authorize HTML handlers replaced by a headless-consent
//! redirect (the RemoteClient owns rendering). Keeps PAR / token /
//! revoke / jwks / well-known metadata routes, plus a PDS-layer `prompt`
//! pre-validation per the OIDC spec.

use super::{SharedOAuthProvider, now_secs};
use cacos_pds_core::config::OAuthRemoteConfig;
use cacos_pds_core::db;
use poem::web::{Data, Form};
use poem::{Request, Response};
use rsky_oauth::client::ParRequest;
use rsky_oauth::request::{REQUEST_URI_PREFIX, request_id_from_uri};
use rsky_oauth::{ClientCredentials, OAuthError, TokenRequest};
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Request material + DPoP helper
// ---------------------------------------------------------------------------

/// Request material needed to validate DPoP proofs.
pub struct OAuthRequestInfo {
    pub method: String,
    pub uri: String,
    pub dpop_headers: Vec<String>,
}

impl OAuthRequestInfo {
    fn from_request(req: &Request, public_url: &str) -> Self {
        let uri = format!(
            "{public_url}{}",
            req.uri()
                .path_and_query()
                .map(|q| q.as_str())
                .unwrap_or("/")
        );
        OAuthRequestInfo {
            method: req.method().as_str().to_string(),
            uri,
            dpop_headers: req
                .headers()
                .get_all("dpop")
                .iter()
                .filter_map(|v| v.to_str().ok().map(String::from))
                .collect(),
        }
    }

    fn dpop_request<'a>(
        &'a self,
        headers: &'a [&'a str],
        access_token: Option<&'a str>,
    ) -> rsky_oauth::dpop::DpopRequest<'a> {
        rsky_oauth::dpop::DpopRequest {
            method: &self.method,
            uri: &self.uri,
            dpop_headers: headers,
            access_token,
        }
    }
}

// ---------------------------------------------------------------------------
// JSON responder (PAR/token/revoke)
// ---------------------------------------------------------------------------

/// Builds a poem response with the given status + JSON body, attaching the
/// DPoP-Nonce header and RFC 6749 cache directives when `nonce` is present.
fn api_response(status: poem::http::StatusCode, body: Value, nonce: Option<String>) -> Response {
    let mut resp = Response::builder()
        .status(status)
        .content_type("application/json")
        .header("Cache-Control", "no-store")
        .header("Pragma", "no-cache")
        .body(body.to_string());
    if let Some(nonce) = nonce {
        resp.headers_mut()
            .insert("DPoP-Nonce", nonce.parse().unwrap());
        resp.headers_mut().insert(
            "Access-Control-Expose-Headers",
            "DPoP-Nonce, WWW-Authenticate".parse().unwrap(),
        );
    }
    resp
}

fn api_ok(body: Value, nonce: Option<String>) -> Response {
    api_response(poem::http::StatusCode::OK, body, nonce)
}

fn api_created(body: Value, nonce: Option<String>) -> Response {
    api_response(poem::http::StatusCode::CREATED, body, nonce)
}

fn api_error(error: OAuthError, nonce: Option<String>) -> Response {
    api_response(
        poem::http::StatusCode::from_u16(error.status())
            .unwrap_or(poem::http::StatusCode::BAD_REQUEST),
        error.to_json(),
        nonce,
    )
}

// ---------------------------------------------------------------------------
// PAR form data
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct ParFormData {
    pub client_id: Option<String>,
    pub response_type: Option<String>,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub login_hint: Option<String>,
    pub prompt: Option<String>,
    pub client_assertion_type: Option<String>,
    pub client_assertion: Option<String>,
}

impl ParFormData {
    fn credentials(&self) -> ClientCredentials {
        ClientCredentials {
            client_id: self.client_id.clone().unwrap_or_default(),
            client_assertion_type: self.client_assertion_type.clone(),
            client_assertion: self.client_assertion.clone(),
        }
    }

    fn par_request(&self) -> ParRequest {
        ParRequest {
            client_id: self.client_id.clone().unwrap_or_default(),
            response_type: self.response_type.clone().unwrap_or_default(),
            response_mode: None,
            redirect_uri: self.redirect_uri.clone(),
            scope: self.scope.clone(),
            state: self.state.clone(),
            code_challenge: self.code_challenge.clone(),
            code_challenge_method: self.code_challenge_method.clone(),
            login_hint: self.login_hint.clone(),
            prompt: self.prompt.clone(),
        }
    }
}

/// PDS-layer prompt pre-validation: reject values outside the advertised
/// `prompt_values_supported` set {none, consent, create} with 400
/// `invalid_request`, per the OIDC spec. The reference silently defaulted
/// unknown prompts to consent — we surface the spec error instead.
#[allow(clippy::result_large_err)]
fn validate_prompt(prompt: Option<&str>) -> Result<(), Response> {
    match prompt {
        None | Some("none") | Some("consent") | Some("create") => Ok(()),
        Some(_) => Err(api_error(
            OAuthError::InvalidRequest(
                "unsupported prompt value; supported: none, consent, create".to_string(),
            ),
            None,
        )),
    }
}

#[poem::handler]
pub async fn oauth_par(
    form: Form<ParFormData>,
    req: &Request,
    shared: Data<&SharedOAuthProvider>,
    config: Data<&crate::OAuthConfig>,
) -> Response {
    let provider = &shared.provider;
    let now = now_secs();
    let nonce = provider.next_dpop_nonce(now);
    if let Err(resp) = validate_prompt(form.prompt.as_deref()) {
        return resp;
    }
    let info = OAuthRequestInfo::from_request(req, &config.public_url);
    let headers: Vec<&str> = info.dpop_headers.iter().map(String::as_str).collect();
    match provider
        .pushed_authorization_request(
            &form.credentials(),
            &form.par_request(),
            &info.dpop_request(&headers, None),
            now,
        )
        .await
    {
        Ok(response) => api_created(
            serde_json::to_value(response).expect("PAR response serialization cannot fail"),
            nonce,
        ),
        Err(error) => api_error(error, nonce),
    }
}

// ---------------------------------------------------------------------------
// Token / revoke
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct TokenFormData {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub client_assertion_type: Option<String>,
    pub client_assertion: Option<String>,
}

#[poem::handler]
pub async fn oauth_token(
    form: Form<TokenFormData>,
    req: &Request,
    shared: Data<&SharedOAuthProvider>,
    config: Data<&crate::OAuthConfig>,
) -> Response {
    let provider = &shared.provider;
    let now = now_secs();
    let nonce = provider.next_dpop_nonce(now);
    let info = OAuthRequestInfo::from_request(req, &config.public_url);
    let headers: Vec<&str> = info.dpop_headers.iter().map(String::as_str).collect();
    let credentials = ClientCredentials {
        client_id: form.client_id.clone().unwrap_or_default(),
        client_assertion_type: form.client_assertion_type.clone(),
        client_assertion: form.client_assertion.clone(),
    };
    let request = TokenRequest {
        grant_type: form.grant_type.clone().unwrap_or_default(),
        code: form.code.clone(),
        redirect_uri: form.redirect_uri.clone(),
        code_verifier: form.code_verifier.clone(),
        refresh_token: form.refresh_token.clone(),
    };
    match provider
        .token(
            &credentials,
            &request,
            &info.dpop_request(&headers, None),
            now,
        )
        .await
    {
        Ok(response) => api_ok(
            serde_json::to_value(response).expect("token response serialization cannot fail"),
            nonce,
        ),
        Err(error) => api_error(error, nonce),
    }
}

#[derive(Deserialize, Default)]
pub struct RevokeFormData {
    pub token: Option<String>,
    pub client_id: Option<String>,
    pub client_assertion_type: Option<String>,
    pub client_assertion: Option<String>,
}

#[poem::handler]
pub async fn oauth_revoke(
    form: Form<RevokeFormData>,
    shared: Data<&SharedOAuthProvider>,
) -> Response {
    let provider = &shared.provider;
    let now = now_secs();
    let nonce = provider.next_dpop_nonce(now);
    let credentials = ClientCredentials {
        client_id: form.client_id.clone().unwrap_or_default(),
        client_assertion_type: form.client_assertion_type.clone(),
        client_assertion: form.client_assertion.clone(),
    };
    let Some(token) = form.token.clone() else {
        return api_error(
            OAuthError::InvalidRequest("token is required".to_string()),
            nonce,
        );
    };
    match provider.revoke(&credentials, &token, now).await {
        Ok(()) => api_ok(serde_json::json!({}), nonce),
        Err(error) => api_error(error, nonce),
    }
}

// ---------------------------------------------------------------------------
// JWKS + well-known metadata
// ---------------------------------------------------------------------------

#[poem::handler]
pub async fn oauth_jwks(shared: Data<&SharedOAuthProvider>) -> Response {
    Response::builder().content_type("application/json").body(
        serde_json::to_string(&shared.provider.jwks()).expect("jwks serialization cannot fail"),
    )
}

#[poem::handler]
pub async fn oauth_authorization_server_metadata(shared: Data<&SharedOAuthProvider>) -> Response {
    Response::builder().content_type("application/json").body(
        serde_json::to_string(&shared.provider.authorization_server_metadata())
            .expect("metadata serialization cannot fail"),
    )
}

#[poem::handler]
pub async fn oauth_protected_resource_metadata(shared: Data<&SharedOAuthProvider>) -> Response {
    Response::builder().content_type("application/json").body(
        serde_json::to_string(&shared.provider.protected_resource_metadata())
            .expect("metadata serialization cannot fail"),
    )
}

// ---------------------------------------------------------------------------
// Authorize redirect (headless consent)
// ---------------------------------------------------------------------------

/// GET /oauth/authorize/:client_id/:request_uri
///
/// Validates the `request_uri`, mints a consent_state nonce, and 302s the
/// browser to the configured RemoteClient. When the RemoteClient is not
/// configured, returns 503 (no dev fallback per the headless-consent spec).
#[poem::handler]
pub async fn oauth_authorize(
    poem::web::Path((_client_id, request_uri)): poem::web::Path<(String, String)>,
    remote: Data<&OAuthRemoteConfig>,
    db: Data<&sea_orm::DatabaseConnection>,
) -> Response {
    let (Some(url), Some(_token)) = (remote.url.as_ref(), remote.token.as_ref()) else {
        return Response::builder()
            .status(poem::http::StatusCode::SERVICE_UNAVAILABLE)
            .content_type("text/plain; charset=utf-8")
            .body("PDS_OAUTH_REMOTE_CLIENT_URL not configured");
    };
    if !request_uri.starts_with(REQUEST_URI_PREFIX) {
        return Response::builder()
            .status(poem::http::StatusCode::BAD_REQUEST)
            .content_type("text/plain; charset=utf-8")
            .body("invalid request_uri");
    }
    let request_id = match request_id_from_uri(&request_uri) {
        Ok(id) => id,
        Err(_) => {
            return Response::builder()
                .status(poem::http::StatusCode::BAD_REQUEST)
                .content_type("text/plain; charset=utf-8")
                .body("invalid request_uri");
        }
    };
    let nonce = match db::consent_state::create(&db, request_id).await {
        Ok(s) => s,
        Err(_) => {
            return Response::builder()
                .status(poem::http::StatusCode::INTERNAL_SERVER_ERROR)
                .content_type("text/plain; charset=utf-8")
                .body("nonce creation failed");
        }
    };
    let redirect = format!("{url}?rqid={request_id}&state={nonce}");
    Response::builder()
        .status(poem::http::StatusCode::FOUND)
        .header("Location", redirect)
        .finish()
}

// ---------------------------------------------------------------------------
// Route assembly
// ---------------------------------------------------------------------------

/// Builds the OAuth provider route set, including the headless-consent
/// redirect and the remote API endpoints (mounted by `mod.rs`).
pub fn oauth_routes() -> poem::Route {
    use poem::{get, post};
    poem::Route::new()
        .at("/oauth/par", post(oauth_par))
        .at("/oauth/token", post(oauth_token))
        .at("/oauth/revoke", post(oauth_revoke))
        .at("/oauth/jwks", get(oauth_jwks))
        .at(
            "/.well-known/oauth-authorization-server",
            get(oauth_authorization_server_metadata),
        )
        .at(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource_metadata),
        )
        .at(
            "/oauth/authorize/:client_id/:request_uri",
            get(oauth_authorize),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cacos_pds_core::config::OAuthRemoteConfig;
    use cacos_pds_core::db::DatabaseKind;
    use poem::EndpointExt;
    use poem::test::TestClient;
    use sea_orm::EntityTrait;

    fn authorize_route_with(
        db: sea_orm::DatabaseConnection,
    ) -> impl poem::Endpoint<Output = poem::Response> {
        let config = OAuthRemoteConfig {
            url: Some("https://remote.example.com".to_string()),
            token: Some("tok".to_string()),
        };
        poem::Route::new()
            .at(
                "/oauth/authorize/:client_id/:request_uri",
                poem::get(oauth_authorize),
            )
            .data(db)
            .data(config)
    }

    #[tokio::test]
    async fn authorize_redirects_to_remote_client() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = DatabaseKind::Account
            .open(dir.path().join("account.sqlite"))
            .await
            .unwrap();
        let app = authorize_route_with(db.clone());
        let client = TestClient::new(app);
        // A well-formed request_uri. rsky-oauth request ids are
        // `req-<32 hex>`; `request_id_from_uri` validates the format.
        let request_uri = "urn:ietf:params:oauth:request_uri:req-0123456789abcdef0123456789abcdef";
        let resp = client
            .get(format!(
                "/oauth/authorize/https%3A%2F%2Fapp.example.com%2Fclient/{request_uri}"
            ))
            .send()
            .await;
        resp.assert_status(poem::http::StatusCode::FOUND);
        // We can't read headers easily; just assert a location redirect by
        // the status code. The consent_state row is created.
        let rows = cacos_pds_core::db::entities::consent_state::Entity::find()
            .all(&db)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn authorize_503_when_remote_not_configured() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = DatabaseKind::Account
            .open(dir.path().join("account.sqlite"))
            .await
            .unwrap();
        let app = poem::Route::new()
            .at(
                "/oauth/authorize/:client_id/:request_uri",
                poem::get(oauth_authorize),
            )
            .data(db)
            .data(OAuthRemoteConfig::default());
        let client = TestClient::new(app);
        let resp = client
            .get("/oauth/authorize/x/urn%3Aietf%3Aparams%3Aoauth%3Arequest_uri%3Areq-0123456789abcdef0123456789abcdef")
            .send()
            .await;
        resp.assert_status(poem::http::StatusCode::SERVICE_UNAVAILABLE);
    }
}
