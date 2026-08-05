//! Headless-consent remote API (poem).
//!
//! The PDS keeps `rsky-oauth`'s `OAuthProvider` unchanged; a single
//! configured RemoteClient owns rendering and calls these
//! token-authenticated JSON endpoints for every consent step. The PDS
//! relays the RemoteClient-supplied `device_id` into every provider
//! decision method.
//!
//! See the [headless-consent design spec](../../docs/superpowers/specs/2026-08-04-headless-oauth-consent-design.md).

use crate::config::OAuthRemoteConfig;
use crate::oauth::remote_create_account::{
    CreateAccountError, CreateAccountInput, RemoteCreateAccount,
};
use crate::oauth::{SharedOAuthProvider, now_secs};
use poem::Response;
use poem::web::{Data, Json, Query};
use rsky_oauth::OAuthError;
use rsky_oauth::request::{AUTHORIZATION_INACTIVITY_TIMEOUT, request_uri_from_id};
use rsky_oauth::types::AuthorizationRequestParameters;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PagePayload {
    pub screen: &'static str,
    pub client: ClientInfo,
    pub scopes: Vec<String>,
    pub login_hint: Option<String>,
    pub prompt: Option<String>,
    pub sessions: Vec<SessionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

#[derive(Serialize)]
pub struct ClientInfo {
    pub id: String,
    pub name: Option<String>,
    pub uri: Option<String>,
    pub logo_uri: Option<String>,
    pub trusted: bool,
}

#[derive(Serialize, Clone)]
pub struct SessionInfo {
    pub did: String,
    pub handle: Option<String>,
    pub email: Option<String>,
}

#[derive(Serialize)]
pub struct RedirectPayload {
    pub redirect_url: String,
}

#[derive(Deserialize)]
pub struct StateQuery {
    pub rqid: String,
    pub state: String,
    pub device_id: String,
}

#[derive(Deserialize)]
pub struct SignInBody {
    pub rqid: String,
    pub state: String,
    pub device_id: String,
    pub identifier: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct SelectBody {
    pub rqid: String,
    pub state: String,
    pub device_id: String,
    pub did: String,
}

#[derive(Deserialize)]
pub struct CreateAccountBody {
    pub rqid: String,
    pub state: String,
    pub device_id: String,
    pub handle: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Deserialize)]
pub struct AcceptBody {
    pub rqid: String,
    pub state: String,
    pub device_id: String,
    pub did: String,
}

#[derive(Deserialize)]
pub struct RejectBody {
    pub rqid: String,
    pub state: String,
    pub device_id: String,
}

// ---------------------------------------------------------------------------
// Token guard
// ---------------------------------------------------------------------------

/// Constant-time bearer token guard against `OAuthRemoteConfig.token`.
pub struct TokenGuard;

impl<'a> poem::FromRequest<'a> for TokenGuard {
    async fn from_request(
        req: &'a poem::Request,
        _body: &mut poem::RequestBody,
    ) -> Result<Self, poem::Error> {
        let auth = req
            .header("Authorization")
            .and_then(|v| v.strip_prefix("Bearer "));
        let expected = req
            .data::<OAuthRemoteConfig>()
            .and_then(|c| c.token.as_deref());
        match (auth, expected) {
            (Some(supplied), Some(expected))
                if constant_time_eq(supplied.as_bytes(), expected.as_bytes()) =>
            {
                Ok(TokenGuard)
            }
            _ => Err(poem::Error::from_string(
                "invalid token",
                poem::http::StatusCode::UNAUTHORIZED,
            )),
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn oauth_error_to_poem(error: OAuthError) -> poem::Error {
    poem::Error::from_response(
        Response::builder()
            .status(
                poem::http::StatusCode::from_u16(error.status())
                    .unwrap_or(poem::http::StatusCode::BAD_REQUEST),
            )
            .content_type("application/json")
            .body(error.to_json().to_string()),
    )
}

/// Read + validate the authorization request row, binding it to the
/// device when not yet bound. Mirrors `OAuthProvider::active_request` at
/// the PDS layer using the shared store (rsky-oauth keeps
/// `active_request` private).
async fn read_request_data(
    shared: &SharedOAuthProvider,
    request_id: &str,
    device_id: &str,
    now: u64,
) -> Result<(String, AuthorizationRequestParameters), OAuthError> {
    let mut data = shared
        .provider
        .store()
        .read_request(request_id)
        .await?
        .ok_or_else(|| OAuthError::InvalidRequest("unknown request_uri".into()))?;
    if data.is_authorized() {
        return Err(OAuthError::InvalidGrant("already authorized".into()));
    }
    if data.is_expired(now) {
        return Err(OAuthError::InvalidGrant("expired".into()));
    }
    match &data.device_id {
        None => data.device_id = Some(device_id.to_owned()),
        Some(bound) if bound != device_id => {
            return Err(OAuthError::InvalidGrant("another device".into()));
        }
        _ => {}
    }
    data.expires_at = now + AUTHORIZATION_INACTIVITY_TIMEOUT;
    shared
        .provider
        .store()
        .update_request(request_id, &data)
        .await?;
    let client_id = data.client_id.clone();
    Ok((client_id, data.parameters.clone()))
}

fn compute_screen(
    data: &AuthorizationRequestParameters,
    _page: &rsky_oauth::provider::AuthorizePageData,
    sessions: &[SessionInfo],
) -> &'static str {
    if data.prompt.as_deref() == Some("create") {
        return "create";
    }
    if sessions.is_empty() {
        return "sign-in";
    }
    if data.prompt.as_deref() == Some("select_account") && sessions.len() > 1 {
        return "select";
    }
    "consent"
}

fn session_info(account: &rsky_oauth::store::AccountInfo) -> SessionInfo {
    SessionInfo {
        did: account.did.clone(),
        handle: account.handle.clone(),
        email: account.email.clone(),
    }
}

fn consent_page(
    page: &rsky_oauth::provider::AuthorizePageData,
    sessions: &[SessionInfo],
    state: Option<String>,
) -> PagePayload {
    PagePayload {
        screen: "consent",
        client: ClientInfo {
            id: page.client_id.clone(),
            name: page.client_name.clone(),
            uri: page.client_uri.clone(),
            logo_uri: page.logo_uri.clone(),
            trusted: page.client_trusted,
        },
        scopes: page.scopes.clone(),
        login_hint: page.login_hint.clone(),
        prompt: page.prompt.clone(),
        sessions: sessions.to_vec(),
        state,
        error: None,
        error_description: None,
    }
}

fn error_page(
    screen: &'static str,
    page: &rsky_oauth::provider::AuthorizePageData,
    sessions: &[SessionInfo],
    state: Option<String>,
    error: OAuthError,
) -> PagePayload {
    PagePayload {
        screen,
        client: ClientInfo {
            id: page.client_id.clone(),
            name: page.client_name.clone(),
            uri: page.client_uri.clone(),
            logo_uri: page.logo_uri.clone(),
            trusted: page.client_trusted,
        },
        scopes: page.scopes.clone(),
        login_hint: page.login_hint.clone(),
        prompt: page.prompt.clone(),
        sessions: sessions.to_vec(),
        state,
        error: Some(error.error_code().to_string()),
        error_description: Some(error.error_description().to_string()),
    }
}

async fn fetch_page(
    shared: &SharedOAuthProvider,
    client_id: &str,
    request_uri: &str,
    device_id: &str,
    now: u64,
) -> Result<(rsky_oauth::provider::AuthorizePageData, Vec<SessionInfo>), OAuthError> {
    let page = shared
        .provider
        .authorize(client_id, request_uri, device_id, now)
        .await?;
    let sessions = page.sessions.iter().map(session_info).collect::<Vec<_>>();
    Ok((page, sessions))
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

#[poem::handler]
pub async fn request(
    Query(q): Query<StateQuery>,
    _t: TokenGuard,
    shared: Data<&SharedOAuthProvider>,
    db: Data<&DatabaseConnection>,
) -> Result<Json<PagePayload>, poem::Error> {
    let now = now_secs();
    let fresh = crate::db::consent_state::rotate(&db, &q.rqid, &q.state)
        .await
        .map_err(|_| {
            poem::Error::from_string("invalid state", poem::http::StatusCode::UNAUTHORIZED)
        })?;
    let (client_id, parameters) = read_request_data(&shared, &q.rqid, &q.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let request_uri = request_uri_from_id(&q.rqid);
    let (page, sessions) = fetch_page(&shared, &client_id, &request_uri, &q.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let screen = compute_screen(&parameters, &page, &sessions);
    Ok(Json(PagePayload {
        screen,
        client: ClientInfo {
            id: page.client_id.clone(),
            name: page.client_name.clone(),
            uri: page.client_uri.clone(),
            logo_uri: page.logo_uri.clone(),
            trusted: page.client_trusted,
        },
        scopes: page.scopes.clone(),
        login_hint: page.login_hint.clone(),
        prompt: page.prompt.clone(),
        sessions,
        state: Some(fresh),
        error: None,
        error_description: None,
    }))
}

#[poem::handler]
pub async fn sign_in(
    Json(b): Json<SignInBody>,
    _t: TokenGuard,
    shared: Data<&SharedOAuthProvider>,
    db: Data<&DatabaseConnection>,
) -> Result<Json<PagePayload>, poem::Error> {
    let now = now_secs();
    let fresh = crate::db::consent_state::rotate(&db, &b.rqid, &b.state)
        .await
        .map_err(|_| {
            poem::Error::from_string("invalid state", poem::http::StatusCode::UNAUTHORIZED)
        })?;
    let (client_id, _parameters) = read_request_data(&shared, &b.rqid, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let request_uri = request_uri_from_id(&b.rqid);
    let sign_result = shared
        .provider
        .sign_in(
            &client_id,
            &request_uri,
            &b.device_id,
            &b.identifier,
            &b.password,
            now,
        )
        .await;
    let (page, sessions) = fetch_page(&shared, &client_id, &request_uri, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    if let Err(err) = sign_result {
        return Ok(Json(error_page(
            "sign-in",
            &page,
            &sessions,
            Some(fresh),
            err,
        )));
    }
    Ok(Json(consent_page(&page, &sessions, Some(fresh))))
}

#[poem::handler]
pub async fn select_account(
    Json(b): Json<SelectBody>,
    _t: TokenGuard,
    shared: Data<&SharedOAuthProvider>,
    db: Data<&DatabaseConnection>,
) -> Result<Json<PagePayload>, poem::Error> {
    let now = now_secs();
    let fresh = crate::db::consent_state::rotate(&db, &b.rqid, &b.state)
        .await
        .map_err(|_| {
            poem::Error::from_string("invalid state", poem::http::StatusCode::UNAUTHORIZED)
        })?;
    let (client_id, _parameters) = read_request_data(&shared, &b.rqid, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let request_uri = request_uri_from_id(&b.rqid);
    let (page, sessions) = fetch_page(&shared, &client_id, &request_uri, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    if !sessions.iter().any(|s| s.did == b.did) {
        return Ok(Json(error_page(
            "select",
            &page,
            &sessions,
            Some(fresh),
            OAuthError::InvalidRequest("did not in sessions".into()),
        )));
    }
    Ok(Json(consent_page(&page, &sessions, Some(fresh))))
}

#[poem::handler]
pub async fn accept(
    Json(b): Json<AcceptBody>,
    _t: TokenGuard,
    shared: Data<&SharedOAuthProvider>,
    db: Data<&DatabaseConnection>,
) -> Result<Json<RedirectPayload>, poem::Error> {
    let now = now_secs();
    let fresh = crate::db::consent_state::rotate(&db, &b.rqid, &b.state)
        .await
        .map_err(|_| {
            poem::Error::from_string("invalid state", poem::http::StatusCode::UNAUTHORIZED)
        })?;
    let _ = fresh;
    let (client_id, _) = read_request_data(&shared, &b.rqid, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let request_uri = request_uri_from_id(&b.rqid);
    let url = shared
        .provider
        .accept(&client_id, &request_uri, &b.device_id, &b.did, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let _ = crate::db::consent_state::delete(&db, &b.rqid).await;
    Ok(Json(RedirectPayload { redirect_url: url }))
}

#[poem::handler]
pub async fn reject(
    Json(b): Json<RejectBody>,
    _t: TokenGuard,
    shared: Data<&SharedOAuthProvider>,
    db: Data<&DatabaseConnection>,
) -> Result<Json<RedirectPayload>, poem::Error> {
    let now = now_secs();
    let _fresh = crate::db::consent_state::rotate(&db, &b.rqid, &b.state)
        .await
        .map_err(|_| {
            poem::Error::from_string("invalid state", poem::http::StatusCode::UNAUTHORIZED)
        })?;
    let (client_id, _) = read_request_data(&shared, &b.rqid, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let request_uri = request_uri_from_id(&b.rqid);
    let url = shared
        .provider
        .reject(&client_id, &request_uri, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let _ = crate::db::consent_state::delete(&db, &b.rqid).await;
    Ok(Json(RedirectPayload { redirect_url: url }))
}

#[poem::handler]
pub async fn create_account(
    Json(b): Json<CreateAccountBody>,
    _t: TokenGuard,
    shared: Data<&SharedOAuthProvider>,
    db: Data<&DatabaseConnection>,
    impl_: Data<&Arc<dyn RemoteCreateAccount>>,
) -> Result<Json<PagePayload>, poem::Error> {
    let now = now_secs();
    let fresh = crate::db::consent_state::rotate(&db, &b.rqid, &b.state)
        .await
        .map_err(|_| {
            poem::Error::from_string("invalid state", poem::http::StatusCode::UNAUTHORIZED)
        })?;
    let (client_id, _parameters) = read_request_data(&shared, &b.rqid, &b.device_id, now)
        .await
        .map_err(oauth_error_to_poem)?;
    let request_uri = request_uri_from_id(&b.rqid);

    let result = impl_
        .create_account(CreateAccountInput {
            rqid: b.rqid.clone(),
            request_uri: request_uri.clone(),
            client_id: client_id.clone(),
            device_id: b.device_id.clone(),
            handle: b.handle.clone(),
            email: b.email.clone(),
            password: b.password.clone(),
            invite_code: b.invite_code.clone(),
        })
        .await;

    match result {
        Ok(_new_did) => {
            let (page, sessions) = fetch_page(&shared, &client_id, &request_uri, &b.device_id, now)
                .await
                .map_err(oauth_error_to_poem)?;
            Ok(Json(consent_page(&page, &sessions, Some(fresh))))
        }
        Err(CreateAccountError::OAuth(e)) => {
            let (page, sessions) = fetch_page(&shared, &client_id, &request_uri, &b.device_id, now)
                .await
                .map_err(oauth_error_to_poem)?;
            Ok(Json(error_page("create", &page, &sessions, Some(fresh), e)))
        }
        Err(CreateAccountError::Internal(msg)) => Err(poem::Error::from_string(
            msg,
            poem::http::StatusCode::INTERNAL_SERVER_ERROR,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DatabaseKind;
    use poem::EndpointExt;
    use poem::test::TestClient;
    use rsky_oauth::store::MemoryOAuthStore;
    use rsky_oauth::{OAuthProvider, OAuthProviderConfig};
    use std::sync::Arc;

    const TEST_KEY_HEX: &str = "4242424242424242424242424242424242424242424242424242424242424242";
    const ISSUER: &str = "https://pds.test";
    const AUDIENCE: &str = "did:web:pds.test";

    fn init_env() {
        let defaults = [
            ("PDS_JWT_KEY_K256_PRIVATE_KEY_HEX", TEST_KEY_HEX),
            ("PDS_SERVICE_DID", AUDIENCE),
        ];
        for (key, value) in defaults {
            // SAFETY: tests run sequentially.
            unsafe { std::env::set_var(key, value) };
        }
    }

    fn provider() -> Arc<OAuthProvider> {
        let store = Arc::new(MemoryOAuthStore::new());
        let key = rsky_oauth::Jwk::from_private_key_bytes(
            rsky_oauth::EcCurve::K256,
            &hex::decode(TEST_KEY_HEX).unwrap(),
        )
        .unwrap();
        Arc::new(OAuthProvider::new(OAuthProviderConfig {
            issuer: ISSUER.to_string(),
            audience: AUDIENCE.to_string(),
            signing_key: key,
            fetcher: Arc::new(crate::oauth::fetcher::HttpClientMetadataFetcher::new()),
            store,
            dpop: rsky_oauth::DpopManager::new(
                None,
                Box::new(rsky_oauth::InMemoryReplayStore::default()),
            ),
            trusted_clients: vec![],
        }))
    }

    #[tokio::test]
    async fn request_requires_token() {
        init_env();
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db = DatabaseKind::Account
            .open(dir.path().join("account.sqlite"))
            .await
            .unwrap();
        let shared = SharedOAuthProvider {
            provider: provider(),
        };
        let config = OAuthRemoteConfig {
            url: Some("https://remote.example.com".into()),
            token: Some("secret-token".into()),
        };
        let app = poem::Route::new()
            .at("/oauth/remote/request", poem::get(request))
            .data(shared)
            .data(db)
            .data(config);
        let client = TestClient::new(app);
        // No auth header -> 401.
        let resp = client
            .get("/oauth/remote/request?rqid=req-0123456789abcdef0123456789abcdef&state=xyz&device_id=dev-1")
            .send()
            .await;
        resp.assert_status(poem::http::StatusCode::UNAUTHORIZED);
        // Wrong token -> 401.
        let resp = client
            .get("/oauth/remote/request?rqid=req-0123456789abcdef0123456789abcdef&state=xyz&device_id=dev-1")
            .header("Authorization", "Bearer wrong")
            .send()
            .await;
        resp.assert_status(poem::http::StatusCode::UNAUTHORIZED);
    }
}
