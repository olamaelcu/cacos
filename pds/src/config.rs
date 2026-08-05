//! Server configuration loaded from the environment.

use std::env;

/// Headless-consent RemoteClient configuration.
///
/// A single operator-configured RemoteClient owns rendering for the OAuth
/// authorize/sign-in/consent/error screens; the PDS exposes a
/// token-authenticated JSON API instead. Unset `url`/`token` disables the
/// `/oauth/authorize` redirect (it returns 503).
#[derive(Debug, Clone, Default)]
pub struct OAuthRemoteConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

impl OAuthRemoteConfig {
    pub fn from_env() -> Self {
        let url = env::var("PDS_OAUTH_REMOTE_CLIENT_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let token = env::var("PDS_OAUTH_REMOTE_CLIENT_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        Self { url, token }
    }
}
